// 등록된 프로젝트의 NAS 문서 폴더를 제한 시간 안에 Windows 탐색기로 연다.
use super::{err, KnowledgeService, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    process::Command,
    time::Duration,
};

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    project_id: String,
    source_id: String,
    shortcut: String,
}

impl KnowledgeService {
    pub fn resolve_project_nas_folder(
        &self,
        project: &str,
        source: &str,
        shortcut: &str,
    ) -> Result<PathBuf> {
        self.require_project(project)?;
        let config = self.get_document_config()?;
        let entry = config["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| {
                entry["sourceId"] == source
                    && entry["scope"] == "project"
                    && entry["projectId"] == project
                    && entry["enabled"] == true
                    && entry["archived"] != true
                    && entry["location"] == "nas"
            })
            .ok_or("이 프로젝트에 등록한 NAS 문서 폴더가 없습니다.")?;
        let root = PathBuf::from(entry["root"].as_str().ok_or("NAS 경로가 비어 있습니다.")?);
        if !super::publish_files::is_unc(&root)
            || root
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
        {
            return Err("등록한 UNC 공유 폴더 경로를 확인해 주세요.".into());
        }
        match shortcut {
            "root" => Ok(root),
            "dailyLog" => Ok(root.join("일자별 작업기록")),
            "checklist" => Ok(root.join("개발 진행 체크리스트")),
            "review" => Ok(root.join("검토·검증 결과")),
            _ => Err("등록된 문서 바로가기를 선택해 주세요.".into()),
        }
    }
    pub fn open_project_nas_folder(
        &self,
        project: String,
        source: String,
        shortcut: String,
    ) -> Result<Value> {
        self.resolve_project_nas_folder(&project, &source, &shortcut)?;
        let mut command = Command::new(self.worker_executable()?);
        command
            .arg("--nas-folder-worker")
            .arg(self.engine.data_dir());
        let input = serde_json::to_vec(&Request {
            project_id: project,
            source_id: source,
            shortcut,
        })
        .map_err(err)?;
        let output =
            super::report_worker::capture(command, Some(input), 16 * 1024, Duration::from_secs(12))
                .map_err(|_| "NAS 폴더 응답 시간이 초과되었습니다. 연결을 확인해 주세요.")?;
        let response: Value = serde_json::from_slice(&output.bytes).map_err(err)?;
        if response["ok"] == true {
            Ok(response["data"].clone())
        } else {
            Err(response["error"]
                .as_str()
                .unwrap_or("NAS 폴더를 열지 못했습니다.")
                .into())
        }
    }
}
#[cfg(windows)]
fn explore(path: &Path) -> Result<()> {
    use std::os::windows::{ffi::OsStrExt, fs::MetadataExt};
    use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
    let info =
        fs::symlink_metadata(path).map_err(|e| format!("NAS 폴더에 접근할 수 없습니다. {e}"))?;
    if !info.is_dir() || info.file_attributes() & 0x400 != 0 {
        return Err("일반 NAS 폴더를 선택해 주세요.".into());
    }
    let verb: Vec<_> = "explore".encode_utf16().chain(Some(0)).collect();
    let wide: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            wide.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if result as isize <= 32 {
        return Err(format!(
            "Windows 탐색기에서 열 수 없습니다. ({})",
            result as isize
        ));
    }
    Ok(())
}
#[cfg(not(windows))]
fn explore(_path: &Path) -> Result<()> {
    Err("NAS 폴더 열기는 Windows 앱에서 지원합니다.".into())
}

pub fn worker_main(data: &Path) {
    let result: Result<Value> = (|| {
        let mut bytes = vec![];
        std::io::stdin()
            .take(4097)
            .read_to_end(&mut bytes)
            .map_err(err)?;
        if bytes.len() > 4096 {
            return Err("잘못된 폴더 열기 요청입니다.".into());
        }
        let r: Request = serde_json::from_slice(&bytes).map_err(err)?;
        let engine = crate::service::Engine::connect(data.to_owned())?;
        let path = engine.knowledge().resolve_project_nas_folder(
            &r.project_id,
            &r.source_id,
            &r.shortcut,
        )?;
        explore(&path)?;
        Ok(json!({"state":"opened","sourceId":r.source_id,"shortcut":r.shortcut,"path":path}))
    })();
    let response = match result {
        Ok(value) => json!({"ok":true,"data":value}),
        Err(error) => json!({"ok":false,"error":error}),
    };
    println!("{response}");
}
