#![forbid(unsafe_code)]

use rmux_harness::{Category, ResultEntry, extract, manifest, map_namespace, run_test, validate};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, env, fs, io, path::PathBuf, process::ExitCode, time::Duration};

#[derive(Serialize, Deserialize)]
struct Report {
    pin: String,
    run_date: String,
    platform: String,
    binary: String,
    mode: String,
    #[serde(default)]
    timeout_seconds: u64,
    summary: BTreeMap<String, usize>,
    tests: Vec<ResultEntry>,
}
fn run() -> Result<bool, Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let mut binary = None;
    let mut source = env::var_os("TMUX_SRC")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env::var_os("HOME").unwrap_or_default()).join("fun/tmux"));
    let mut json = PathBuf::from("harness/runs/latest.json");
    let mut baseline = None;
    let mut rmux = false;
    let mut timeout = 120;
    let mut selected = Vec::new();
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next()
                .ok_or_else(|| io::Error::other(format!("missing value for {arg}")))
        };
        match arg.as_str() {
            "--binary" => binary = Some(PathBuf::from(value()?)),
            "--source" => source = PathBuf::from(value()?),
            "--json" => json = PathBuf::from(value()?),
            "--baseline" => baseline = Some(PathBuf::from(value()?)),
            "--timeout" => timeout = value()?.parse()?,
            "--test" => selected.push(value()?),
            "--rmux" => rmux = true,
            "--help" => {
                println!(
                    "regress --binary PATH [--source TMUX_REPO] [--rmux] [--baseline JSON] [--test NAME] [--timeout SECONDS] [--json PATH]"
                );
                return Ok(true);
            }
            _ => return Err(io::Error::other(format!("unknown argument: {arg}")).into()),
        }
    }
    if timeout == 0 {
        return Err(io::Error::other("timeout must be positive").into());
    }
    let binary = fs::canonicalize(binary.ok_or_else(|| io::Error::other("--binary required"))?)?;
    let temp_root = env::var_os("RMUX_TEST_TMPDIR").unwrap_or_else(|| "/tmp".into());
    let root = tempfile::Builder::new()
        .prefix("rr-")
        .tempdir_in(temp_root)?;
    extract(&source, root.path())?;
    let regress = root.path().join("regress");
    let manifest = manifest()?;
    validate(&manifest, &regress)?;
    for name in &selected {
        if !manifest.tests.iter().any(|test| &test.name == name) {
            return Err(io::Error::other(format!("unknown test {name}")).into());
        }
    }
    if rmux {
        for entry in fs::read_dir(&regress)? {
            let path = entry?.path();
            let is_include = path.extension().and_then(|s| s.to_str()) == Some("inc");
            let is_mapped = manifest.tests.iter().any(|test| {
                test.category == Category::Mapped
                    && path.file_name().and_then(|name| name.to_str()) == Some(&test.name)
            });
            if is_include || is_mapped {
                fs::write(&path, map_namespace(&fs::read_to_string(&path)?))?;
            }
        }
    }
    let baseline: Option<Report> = baseline
        .map(|path| {
            fs::read(path)
                .and_then(|bytes| serde_json::from_slice(&bytes).map_err(io::Error::other))
        })
        .transpose()?;
    let mut tests = Vec::new();
    for test in &manifest.tests {
        if !selected.is_empty() && !selected.contains(&test.name) {
            continue;
        }
        if rmux && test.category == Category::Mapped {
            println!("mapped: {}", test.name);
        }
        let work = root.path().join(format!("t{}", tests.len()));
        let mut result = run_test(
            &binary,
            &regress.join(&test.name),
            &work,
            Duration::from_secs(timeout),
            rmux,
            test,
        )?;
        if rmux
            && result.status != "needs-fixture"
            && baseline.as_ref().is_some_and(|report| {
                report.tests.iter().any(|entry| {
                    entry.name == test.name
                        && matches!(entry.status.as_str(), "fail" | "timeout" | "oracle-fail")
                })
            })
        {
            result.status = "oracle-fail".into();
        }
        println!(
            "{}: {} ({} ms){}",
            result.name,
            result.status,
            result.elapsed_ms,
            if result.status == "fail" && result.stderr.contains("server not implemented yet") {
                " — rmux P0 server not implemented"
            } else {
                ""
            }
        );
        tests.push(result);
    }
    let mut summary = BTreeMap::new();
    for test in &tests {
        *summary.entry(test.status.clone()).or_insert(0) += 1;
    }
    println!("summary: {}", serde_json::to_string(&summary)?);
    let success = !tests
        .iter()
        .any(|test| matches!(test.status.as_str(), "fail" | "timeout"));
    let date = std::process::Command::new("date")
        .args(["-u", "+%Y-%m-%dT%H:%M:%SZ"])
        .output()?;
    let report = Report {
        pin: rmux_harness::PIN.trim().into(),
        run_date: String::from_utf8(date.stdout)?.trim().into(),
        platform: format!("{}-{}", env::consts::OS, env::consts::ARCH),
        binary: binary.display().to_string(),
        mode: if rmux { "rmux" } else { "oracle" }.into(),
        timeout_seconds: timeout,
        summary,
        tests,
    };
    if let Some(parent) = json.parent().filter(|path| !path.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    fs::write(json, serde_json::to_string_pretty(&report)? + "\n")?;
    Ok(success)
}
fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("regress: {error}");
            ExitCode::FAILURE
        }
    }
}
