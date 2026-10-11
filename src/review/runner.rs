//! 明示された判定器だけを起動する。シェルの展開、暗黙の外部送信は行わない。

use process_wrap::std::CommandWrap;
#[cfg(windows)]
use process_wrap::std::JobObject;
#[cfg(unix)]
use process_wrap::std::ProcessGroup;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const OUTPUT_LIMIT: u64 = 8 * 1024 * 1024;

enum Completed {
    Input(Result<(), std::io::Error>),
    Output(Result<Vec<u8>, std::io::Error>),
    Error(Result<Vec<u8>, std::io::Error>),
}

fn read_bounded(reader: impl Read) -> Result<Vec<u8>, std::io::Error> {
    let mut bytes = vec![];
    reader.take(OUTPUT_LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > OUTPUT_LIMIT {
        return Err(std::io::Error::other("判定器の出力が上限を超えました"));
    }
    Ok(bytes)
}

pub(super) fn invoke(
    executable: &Path,
    args: &[String],
    input: Vec<u8>,
    seconds: u64,
) -> Result<Vec<u8>, String> {
    let mut command = Command::new(executable);
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut command = CommandWrap::from(command);
    #[cfg(unix)]
    command.wrap(ProcessGroup::leader());
    #[cfg(windows)]
    command.wrap(JobObject);
    let mut child = command
        .spawn()
        .map_err(|e| format!("判定器を起動できません: {e}"))?;
    let mut stdin = child.stdin().take().unwrap();
    let stdout = child.stdout().take().unwrap();
    let stderr = child.stderr().take().unwrap();
    let (tx, rx) = mpsc::channel();
    let send = tx.clone();
    thread::spawn(move || {
        let _ = send.send(Completed::Input(stdin.write_all(&input)));
    });
    let send = tx.clone();
    thread::spawn(move || {
        let _ = send.send(Completed::Output(read_bounded(stdout)));
    });
    thread::spawn(move || {
        let _ = tx.send(Completed::Error(read_bounded(stderr)));
    });
    let deadline = Instant::now() + Duration::from_secs(seconds);
    let mut output = None;
    let mut status = None;
    let mut completed = 0;
    let mut stderr = vec![];
    let mut input_error = None;
    let mut exited_at = None;
    let result = 'wait: loop {
        while let Ok(message) = rx.try_recv() {
            completed += 1;
            let error = match message {
                Completed::Input(result) => {
                    input_error = result.err();
                    None
                }
                Completed::Output(result) => match result {
                    Ok(bytes) => {
                        output = Some(bytes);
                        None
                    }
                    Err(e) => Some(e),
                },
                Completed::Error(result) => match result {
                    Ok(bytes) => {
                        stderr = bytes;
                        None
                    }
                    Err(e) => Some(e),
                },
            };
            if let Some(error) = error {
                break 'wait Err(format!("判定器との入出力に失敗しました: {error}"));
            }
        }
        if status.is_none() {
            match child.try_wait() {
                Ok(value) => status = value,
                Err(e) => break Err(format!("判定器の状態を確認できません: {e}")),
            }
        }
        if let Some(status) = status {
            let exited_at = *exited_at.get_or_insert_with(Instant::now);
            if completed == 3 {
                if !status.success() {
                    let tail =
                        String::from_utf8_lossy(&stderr[stderr.len().saturating_sub(4096)..]);
                    break Err(format!("判定器が失敗しました: {status}\n{tail}"));
                }
                if let Some(error) = input_error {
                    break Err(format!("判定器に依頼を渡せません: {error}"));
                }
                break output.ok_or_else(|| "判定器の標準出力を読めません".into());
            }
            if exited_at.elapsed() >= Duration::from_secs(1) {
                break Err("判定器の終了後も子孫プロセスが入出力を閉じませんでした".into());
            }
        }
        if Instant::now() >= deadline {
            break Err("判定器が制限時間内に完了しませんでした".into());
        }
        thread::sleep(Duration::from_millis(10));
    };
    // 終了した判定器が残した子孫も回収する。タイムアウト時は同じグループ全体を止める。
    let _ = child.kill();
    let _ = child.wait();
    result
}
