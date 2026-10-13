//! Probe the packaged sidecar's stdio pipe path with the same options the
//! orchestrator uses in `engine.rs`: tokio pipes, job-directory cwd, and the
//! unbuffered environment.
//!
//! Skipped unless `OPENDEMAND_ENGINE` is set, so development and the regular
//! test suites are unaffected. The Release gate and the Sidecar Diagnosis
//! workflow set it to the frozen binary.

use std::{path::Path, process::Stdio, time::Duration};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

const REQUEST: &str = r#"{"protocol":"opendemand.sidecar/WRONG","type":"job.request","job_id":"probe-tokio","contract":"opendemand.arrow/1","input_files":{"observations":"observations.arrow","tasks":"tasks.arrow"}}"#;

#[tokio::test]
async fn frozen_sidecar_answers_a_request_over_tokio_pipes() {
    let Some(engine) = std::env::var_os("OPENDEMAND_ENGINE") else {
        println!("OPENDEMAND_ENGINE is not set; skipping the tokio stdio probe.");
        return;
    };
    let engine = std::path::PathBuf::from(engine);
    let directory = tempfile::tempdir().expect("temp job directory");
    let mut child = spawn(&engine, directory.path()).expect("frozen sidecar could not start");
    let mut stdin = child.stdin.take().expect("child stdin");
    let mut stdout = BufReader::new(child.stdout.take().expect("child stdout")).lines();
    let stderr = child.stderr.take().expect("child stderr");
    let stderr_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        let mut collected: Vec<String> = Vec::new();
        while let Ok(Some(line)) = lines.next_line().await {
            collected.push(line);
        }
        collected
    });

    let hello = match tokio::time::timeout(Duration::from_secs(15), stdout.next_line()).await {
        Err(_) => panic!("hello did not arrive within 15s"),
        Ok(Err(error)) => panic!("reading hello failed: {error}"),
        Ok(Ok(None)) => panic!("stdout closed before hello"),
        Ok(Ok(Some(line))) => line,
    };
    assert!(
        hello.contains("\"type\":\"hello\""),
        "unexpected first line: {hello}"
    );

    stdin
        .write_all(format!("{REQUEST}\n").as_bytes())
        .await
        .expect("writing the request failed");
    stdin.flush().await.expect("flushing the request failed");

    let answer = match tokio::time::timeout(Duration::from_secs(15), stdout.next_line()).await {
        Err(_) => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            let evidence = stderr_task.await.unwrap_or_default();
            panic!(
                "frozen sidecar did not answer over the tokio pipes within 15s; engine stderr: {evidence:?}"
            );
        }
        Ok(Err(error)) => panic!("reading the answer failed: {error}"),
        Ok(Ok(None)) => panic!("stdout closed before an answer"),
        Ok(Ok(Some(line))) => line,
    };
    assert!(
        answer.contains("\"type\":\"job.error\"") && answer.contains("protocol_mismatch"),
        "unexpected answer: {answer}"
    );
    println!("tokio stdio probe: request delivered and answered (protocol_mismatch)");

    let status = match tokio::time::timeout(Duration::from_secs(10), child.wait()).await {
        Err(_) => panic!("engine did not exit after job.error"),
        Ok(result) => result.expect("waiting for the engine failed"),
    };
    assert!(
        !status.success(),
        "engine exited cleanly even though it reported job.error"
    );
    println!("tokio stdio probe: engine exited with {status}");
}

fn spawn(engine: &Path, job_dir: &Path) -> std::io::Result<tokio::process::Child> {
    Command::new(engine)
        .arg("--job-dir")
        .arg(job_dir)
        .env("PYTHONUNBUFFERED", "1")
        .env("OPENBLAS_NUM_THREADS", "1")
        .current_dir(job_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
}
