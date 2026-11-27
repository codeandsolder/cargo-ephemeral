//! Materialize Cargo's resolved units on demand for the native executor.
//! The planner owns the layout and build-script semantics, never job admission.
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, ensure};
use cargo_util::{ProcessBuilder, paths};
use sccache_build::{
    BUILD_PLAN_VERSION, BuildPath, BuildPlan, EnvironmentChange, PlanAction, PlanCommand,
    PlanConnection, PlanNode, PlannerReply, PlannerRequest,
};

use super::{BuildRunner, FileFlavor, Unit, custom_build};
use crate::util::CargoResult;

fn path(path: &Path) -> CargoResult<BuildPath> {
    BuildPath::new(path.as_os_str().as_bytes().to_vec())
}

fn absolute(cwd: &Path, p: &Path) -> PathBuf {
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        cwd.join(p)
    }
}

fn executable(
    cwd: &Path,
    env: &BTreeMap<OsString, OsString>,
    program: &OsStr,
) -> CargoResult<PathBuf> {
    let p = Path::new(program);
    if p.components().count() > 1 || p.is_absolute() {
        return Ok(absolute(cwd, p));
    }
    let search = env.get(OsStr::new("PATH")).context("command has no PATH")?;
    std::env::split_paths(search)
        .map(|dir| absolute(cwd, &dir).join(p))
        .find(|candidate| candidate.is_file())
        .with_context(|| format!("executable {program:?} not found in PATH"))
}

fn command(builder: &ProcessBuilder, cwd: &Path, compiler: bool) -> CargoResult<PlanCommand> {
    ensure!(
        builder.get_arg0().is_none(),
        "custom argv[0] is not supported by the native planner"
    );
    let cmd = builder.build_command();
    let cwd = absolute(cwd, cmd.get_current_dir().unwrap_or(cwd));
    let mut env: BTreeMap<OsString, OsString> = std::env::vars_os().collect();
    for (name, value) in cmd.get_envs() {
        if let Some(value) = value {
            env.insert(name.to_owned(), value.to_owned());
        } else {
            env.remove(name);
        }
    }
    let mut args = cmd.get_args();
    let mut program = cmd.get_program();
    if compiler && program != builder.get_program() {
        ensure!(
            matches!(
                Path::new(program).file_name().and_then(OsStr::to_str),
                Some("sccache" | "sccache-client")
            ),
            "unrecognized compiler wrapper {program:?}; native planner cannot bypass it"
        );
        program = args.next().context("missing wrapped compiler executable")?;
        ensure!(
            program == builder.get_program(),
            "nested compiler wrappers are not supported"
        );
    }
    Ok(PlanCommand {
        executable: path(&executable(&cwd, &env, program)?)?,
        cwd: path(&cwd)?,
        arguments: args.map(|arg| arg.as_bytes().to_vec()).collect(),
        inherit_environment: false,
        environment: env
            .into_iter()
            .map(|(name, value)| EnvironmentChange {
                name: name.into_encoded_bytes(),
                value: Some(value.into_encoded_bytes()),
            })
            .collect(),
    })
}

fn required_outputs(runner: &BuildRunner<'_, '_>, unit: &Unit) -> CargoResult<Vec<BuildPath>> {
    if unit.skip_non_compile_time_dep {
        return Ok(Vec::new());
    }
    if unit.mode.is_run_custom_build() {
        return custom_build::plan_script_outputs(runner, unit)
            .iter()
            .map(|p| path(p))
            .collect();
    }
    runner
        .outputs(unit)?
        .iter()
        .filter(|output| {
            matches!(
                output.flavor,
                FileFlavor::Normal | FileFlavor::Linkable | FileFlavor::Rmeta
            )
        })
        .map(|output| path(&output.path))
        .collect()
}

fn prepare(runner: &BuildRunner<'_, '_>, unit: &Unit) -> CargoResult<PlanAction> {
    if unit.skip_non_compile_time_dep {
        return Ok(PlanAction::Noop);
    }
    let cwd = runner.bcx.gctx.cwd();
    if unit.mode.is_run_custom_build() {
        let Some((cmd, stdout, stderr)) = custom_build::plan_script(runner, unit)? else {
            return Ok(PlanAction::Noop);
        };
        return Ok(PlanAction::Process {
            command: command(&cmd, cwd, false)?,
            stdout: Some(path(&stdout)?),
            stderr: Some(path(&stderr)?),
        });
    }
    custom_build::plan_metabuild(runner, unit)?;
    let mut rustc = super::prepare_rustc(runner, unit)?;
    if let Some(scripts) = runner.build_scripts.get(unit) {
        let outputs = runner.build_script_outputs.lock().unwrap();
        super::add_native_deps(
            &mut rustc,
            &outputs,
            scripts,
            unit.target.is_lib() || !unit.pkg.targets().iter().any(|t| t.is_lib()),
            &unit.target,
            unit.pkg.package_id(),
            unit.mode,
        )?;
        if let Some(root) = runner.files().host_dest() {
            super::add_plugin_deps(&mut rustc, &outputs, scripts, root)?;
        }
        super::add_custom_flags(
            &mut rustc,
            &outputs,
            runner.find_build_script_metadatas(unit),
        )?;
    }
    paths::create_dir_all(&runner.files().output_dir(unit))?;
    for output in runner.outputs(unit)?.iter() {
        if output.path.extension() == Some(OsStr::new("rmeta")) {
            let rlib = output.path.with_extension("rlib");
            if rlib.exists() {
                paths::remove_file(&rlib)?;
            }
        }
        if output.hardlink.is_some() && output.path.exists() {
            paths::remove_file(&output.path)?;
        }
    }
    Ok(PlanAction::Compile {
        command: command(&rustc, cwd, true)?,
    })
}

fn complete(runner: &BuildRunner<'_, '_>, unit: &Unit) -> CargoResult<()> {
    if unit.skip_non_compile_time_dep {
        return Ok(());
    }
    if unit.mode.is_run_custom_build() {
        return custom_build::complete_script(runner, unit);
    }
    for output in runner.outputs(unit)?.iter() {
        if !output.path.exists() {
            continue;
        }
        if let Some(dst) = &output.hardlink {
            paths::link_or_copy(&output.path, dst)?;
            if let Some(dst) = &output.export_path {
                if let Some(parent) = dst.parent() {
                    paths::create_dir_all(parent)?;
                }
                paths::link_or_copy(&output.path, dst)?;
            }
        }
    }
    Ok(())
}

pub(super) fn serve(runner: &BuildRunner<'_, '_>) -> CargoResult<()> {
    ensure!(
        !runner.bcx.gctx.cli_unstable().fine_grain_locking,
        "native planner requires exclusive build layout locks"
    );
    let graph = &runner.bcx.unit_graph;
    let mut units = graph.keys().collect::<Vec<_>>();
    units.sort_unstable();
    let indices: BTreeMap<_, _> = units
        .iter()
        .enumerate()
        .map(|(i, unit)| (*unit, i))
        .collect();
    let mut nodes = Vec::with_capacity(units.len());
    for unit in &units {
        ensure!(
            !unit.mode.is_doc() && !unit.mode.is_doc_test() && !unit.mode.is_doc_scrape(),
            "documentation units are not supported"
        );
        ensure!(
            unit.profile.trim_paths.is_none(),
            "native planner does not yet support trim-paths"
        );
        let dependencies = graph[*unit]
            .iter()
            .map(|dep| {
                indices
                    .get(&dep.unit)
                    .copied()
                    .context("unit missing from native plan")
            })
            .collect::<CargoResult<BTreeSet<_>>>()?
            .into_iter()
            .collect();
        nodes.push(PlanNode {
            label: format!(
                "{} {} {:?}",
                unit.pkg.package_id(),
                unit.target.name(),
                unit.mode
            ),
            dependencies,
            estimated_cost: 1,
            directories: Vec::new(),
            required_outputs: required_outputs(runner, unit)?,
            action: PlanAction::Deferred,
        });
    }
    let plan = BuildPlan {
        version: BUILD_PLAN_VERSION,
        nodes,
    };
    plan.graph()?;
    let mut connection = PlanConnection::new(io::stdin().lock(), io::stdout().lock());
    connection.send(&PlannerReply::Plan { plan: plan.clone() })?;
    // 0 = pending, 1 = prepared, 2 = complete.
    let mut state = vec![0u8; units.len()];
    loop {
        let request = connection.receive()?;
        let reply = (|| -> CargoResult<PlannerReply> {
            match request {
                PlannerRequest::Prepare { index } => {
                    ensure!(
                        state.get(index) == Some(&0),
                        "invalid prepare index or state: {index}"
                    );
                    ensure!(
                        plan.nodes[index]
                            .dependencies
                            .iter()
                            .all(|dep| state[*dep] == 2),
                        "node {index} requested before prerequisites completed"
                    );
                    let action = prepare(runner, units[index])?;
                    plan.nodes[index].validate_action(&action)?;
                    state[index] = 1;
                    Ok(PlannerReply::Action { index, action })
                }
                PlannerRequest::Complete { index } => {
                    ensure!(
                        state.get(index) == Some(&1),
                        "invalid completion index or state: {index}"
                    );
                    for output in &plan.nodes[index].required_outputs {
                        ensure!(
                            BuildPlan::path(output).exists(),
                            "node {index} omitted output {:?}",
                            BuildPlan::path(output)
                        );
                    }
                    complete(runner, units[index])?;
                    state[index] = 2;
                    Ok(PlannerReply::Completed { index })
                }
                PlannerRequest::Finish => {
                    ensure!(state.iter().all(|s| *s == 2), "plan is unfinished");
                    Ok(PlannerReply::Finished)
                }
            }
        })();
        match reply {
            Ok(reply) => {
                let finished = matches!(reply, PlannerReply::Finished);
                connection.send(&reply)?;
                if finished {
                    return Ok(());
                }
            }
            Err(err) => {
                connection.send(&PlannerReply::Error {
                    message: format!("{err:#}"),
                })?;
                return Err(err);
            }
        }
    }
}
