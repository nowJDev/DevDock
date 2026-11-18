// 등록된 로컬 핸드오버 폴더에서 보고용 파일명과 날짜를 가져온다.
use super::{err, publish_files::is_unc, KnowledgeService, Result};
use chrono::{DateTime, Local, NaiveDate};
use serde_json::{json, Value};
use std::{fs, path::Path};

fn report_date(name: &str, metadata: &fs::Metadata) -> Option<NaiveDate> {
    name.split(|character: char| !character.is_ascii_digit())
        .filter(|digits| digits.len() == 8)
        .find_map(|digits| NaiveDate::parse_from_str(digits, "%Y%m%d").ok())
        .or_else(|| {
            metadata
                .modified()
                .ok()
                .map(|time| DateTime::<Local>::from(time).date_naive())
        })
}

fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    metadata.file_type().is_symlink()
}

fn list_files(root: &Path) -> Result<Vec<Value>> {
    let metadata = fs::symlink_metadata(root).map_err(err)?;
    if !metadata.is_dir() || is_link(&metadata) {
        return Err("일반 로컬 폴더를 연결해 주세요.".into());
    }
    let mut files = Vec::new();
    for entry in fs::read_dir(root).map_err(err)? {
        let entry = entry.map_err(err)?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if name.chars().any(char::is_control)
            || !matches!(
                Path::new(&name)
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .map(str::to_ascii_lowercase)
                    .as_deref(),
                Some("md" | "txt" | "json" | "snapshot")
            )
        {
            continue;
        }
        let Ok(metadata) = fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if metadata.is_file() && !is_link(&metadata) {
            files.push((report_date(&name, &metadata), name));
        }
    }
    files.sort_by(|(a_date, a_name), (b_date, b_name)| b_date.cmp(a_date).then(a_name.cmp(b_name)));
    Ok(files
        .into_iter()
        .map(|(date, name)| json!({"name":name,"date":date.map(|date| date.to_string())}))
        .collect())
}

impl KnowledgeService {
    pub(crate) fn list_report_handover_files(&self, project: &str) -> Result<Value> {
        self.require_project(project)?;
        let config = self.get_document_config()?;
        let mut folders = Vec::new();
        for source in config["sources"]
            .as_array()
            .ok_or("invalid document sources")?
        {
            if source["enabled"] != true
                || source["archived"] == true
                || source["location"] != "local"
                || !source["roles"]
                    .as_array()
                    .is_some_and(|roles| roles.iter().any(|role| role == "handover"))
                || !(source["scope"] == "global"
                    || (source["scope"] == "project" && source["projectId"] == project))
            {
                continue;
            }
            let Some(root) = source["root"].as_str() else {
                continue;
            };
            let path = Path::new(root);
            if !path.is_absolute() || is_unc(path) {
                continue;
            }
            let mut folder = json!({"sourceId":source["sourceId"],"root":root,"files":[]});
            match list_files(path) {
                Ok(files) => folder["files"] = json!(files),
                Err(error) => folder["error"] = json!(error),
            }
            folders.push(folder);
        }
        Ok(json!({"folders":folders}))
    }
}
