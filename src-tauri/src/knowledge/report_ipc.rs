// 일일보고 초안을 Windows의 로컬 파이프로 앱 메모리에 전달한다.
use super::{err, KnowledgeService, Result};
use serde_json::Value;

#[cfg(windows)]
mod windows {
    use super::*;
    use serde_json::json;
    use std::{ffi::c_void, time::Duration};
    use tokio::{
        io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
        net::windows::named_pipe::{ClientOptions, NamedPipeServer, ServerOptions},
        sync::oneshot,
        time::timeout,
    };
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::{
            Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW,
            SECURITY_ATTRIBUTES,
        },
    };

    const MAX_FRAME: usize = 1024 * 1024;
    const IO_LIMIT: Duration = Duration::from_secs(5);
    const RESULT_LIMIT: Duration = Duration::from_secs(45);

    pub struct Host(Option<oneshot::Sender<()>>);

    impl Drop for Host {
        fn drop(&mut self) {
            if let Some(stop) = self.0.take() {
                let _ = stop.send(());
            }
        }
    }

    fn pipe_name(service: &KnowledgeService) -> Result<String> {
        let directory = std::fs::canonicalize(service.engine.data_dir()).map_err(err)?;
        let digest = crate::knowledge::scanner::sha256(
            directory.to_string_lossy().to_lowercase().as_bytes(),
        );
        Ok(format!(r"\\.\pipe\DevDock.Report.{digest}"))
    }

    fn listen(name: &str, first: bool) -> Result<NamedPipeServer> {
        // OW restricts access to the owner of this pipe. NU also denies network logons.
        let sddl: Vec<_> = "D:P(D;;GA;;;NU)(A;;GA;;;OW)"
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let mut descriptor: *mut c_void = std::ptr::null_mut();
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(err(std::io::Error::last_os_error()));
        }
        let mut security = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        let server = unsafe {
            ServerOptions::new()
                .first_pipe_instance(first)
                .reject_remote_clients(true)
                .max_instances(5)
                .create_with_security_attributes_raw(
                    name,
                    (&mut security as *mut SECURITY_ATTRIBUTES).cast(),
                )
        };
        unsafe { LocalFree(descriptor) };
        server.map_err(err)
    }

    async fn read_frame(stream: &mut (impl AsyncRead + Unpin)) -> Result<Value> {
        let length = stream.read_u32_le().await.map_err(err)? as usize;
        if length == 0 || length > MAX_FRAME {
            return Err("보고 메시지 크기를 확인해 주세요.".into());
        }
        let mut data = vec![0; length];
        stream.read_exact(&mut data).await.map_err(err)?;
        serde_json::from_slice(&data).map_err(err)
    }

    async fn write_frame(stream: &mut (impl AsyncWrite + Unpin), value: &Value) -> Result<()> {
        let data = serde_json::to_vec(value).map_err(err)?;
        if data.len() > MAX_FRAME {
            return Err("보고 메시지 크기를 확인해 주세요.".into());
        }
        stream.write_u32_le(data.len() as u32).await.map_err(err)?;
        stream.write_all(&data).await.map_err(err)
    }

    async fn serve(mut pipe: NamedPipeServer, service: KnowledgeService) -> Result<()> {
        let input = timeout(IO_LIMIT, read_frame(&mut pipe))
            .await
            .map_err(err)??;
        if input["command"] != "submit_report_result" {
            return Err("지원하지 않는 보고 전달 명령입니다.".into());
        }
        let result = tokio::task::spawn_blocking(move || {
            service.execute_command(input, false, true).map(|value| {
                json!({"requestId": value["requestId"], "jobId": value["jobId"], "state": value["state"]})
            })
        }).await.map_err(err)?;
        let response = match result {
            Ok(data) => json!({"ok": true, "data": data}),
            Err(error) => json!({"ok": false, "message": error}),
        };
        timeout(IO_LIMIT, async {
            write_frame(&mut pipe, &response).await?;
            // Keep the server handle open until the client has consumed the reply.
            pipe.read_u8().await.map_err(err)?;
            Ok(())
        })
        .await
        .map_err(err)?
    }

    pub fn start(service: KnowledgeService) -> Result<Host> {
        let name = pipe_name(&service)?;
        let (ready, wait) = std::sync::mpsc::sync_channel(1);
        let (stop, stopping) = oneshot::channel();
        std::thread::Builder::new()
            .name("devdock-report-pipe".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = ready.send(Err(err(error)));
                        return;
                    }
                };
                runtime.block_on(async move {
                    let mut listener = match listen(&name, true) {
                        Ok(listener) => listener,
                        Err(error) => {
                            let _ = ready.send(Err(error));
                            return;
                        }
                    };
                    if ready.send(Ok(())).is_err() {
                        return;
                    }
                    let mut requests = tokio::task::JoinSet::new();
                    tokio::pin!(stopping);
                    loop {
                        tokio::select! {
                            _ = &mut stopping => break,
                            _ = requests.join_next(), if !requests.is_empty() => continue,
                            connected = listener.connect() => {
                                if connected.is_err() { break; }
                                if requests.len() >= 4 {
                                    let _ = listener.disconnect();
                                    continue;
                                }
                                let Ok(next) = listen(&name, false) else { break; };
                                let pipe = std::mem::replace(&mut listener, next);
                                requests.spawn(serve(pipe, service.clone()));
                            }
                        }
                    }
                });
                runtime.shutdown_timeout(IO_LIMIT);
            })
            .map_err(err)?;
        wait.recv_timeout(IO_LIMIT).map_err(err)??;
        Ok(Host(Some(stop)))
    }

    pub fn submit(service: &KnowledgeService, input: Value) -> Result<Value> {
        let name = pipe_name(service)?;
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(err)?
            .block_on(async {
                let mut pipe = ClientOptions::new()
                    .security_qos_flags(0x0001_0000) // SECURITY_IDENTIFICATION
                    .open(&name)
                    .map_err(|_| "DevDock 앱을 열고 보고서를 다시 제출해 주세요.".to_owned())?;
                timeout(IO_LIMIT, write_frame(&mut pipe, &input))
                    .await
                    .map_err(err)??;
                let mut response = timeout(RESULT_LIMIT, read_frame(&mut pipe))
                    .await
                    .map_err(err)??;
                let _ = timeout(IO_LIMIT, pipe.write_u8(1)).await;
                if response["ok"] == true {
                    Ok(response["data"].take())
                } else {
                    Err(response["message"]
                        .as_str()
                        .unwrap_or("보고 전달 실패")
                        .into())
                }
            })
    }
}

#[cfg(windows)]
pub use windows::{start, submit};

#[cfg(not(windows))]
pub fn submit(_service: &KnowledgeService, _input: Value) -> Result<Value> {
    Err("보고 초안 전달은 Windows DevDock 앱에서 지원합니다.".into())
}
