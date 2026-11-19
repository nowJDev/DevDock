// 등록된 로컬 핸드오버 폴더에서 보고용 파일명과 날짜를 가져온다.
use super::{err, publish_files::is_unc, KnowledgeService, Result};
use chrono::{DateTime, Local, NaiveDate};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::{collections::HashSet, fs, path::Path};

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

struct ProjectLinks {
    accepted: HashSet<String>,
    cannot_inherit: HashSet<String>,
}

impl ProjectLinks {
    fn load(db: &Connection, source: &str, project: &str) -> Result<Self> {
        let mut query = db
            .prepare(
                "SELECT d.normalized_path, l.project_id, l.review_state
                 FROM documents d JOIN document_project_links l USING(document_id)
                 WHERE d.source_id=?1 AND (l.review_state='accepted' OR l.project_id=?2)
                   AND (l.review_state!='accepted' OR l.method!='crossChecked' OR EXISTS (
                     SELECT 1 FROM document_versions v WHERE v.document_id=d.document_id
                       AND v.sha256=json_extract(d.payload,'$.sha256')
                       AND json_extract(v.payload,'$.crossCheckedProject')=l.project_id
                   ))",
            )
            .map_err(err)?;
        let mut accepted = HashSet::new();
        let mut cannot_inherit = HashSet::new();
        let mut rows = query.query(params![source, project]).map_err(err)?;
        while let Some(row) = rows.next().map_err(err)? {
            let name: String = row.get(0).map_err(err)?;
            let owner: String = row.get(1).map_err(err)?;
            let state: String = row.get(2).map_err(err)?;
            if owner == project && state == "accepted" {
                accepted.insert(name);
            } else {
                cannot_inherit.insert(name);
            }
        }
        Ok(Self {
            accepted,
            cannot_inherit,
        })
    }

    fn includes(&self, name: &str) -> bool {
        let normalized = name.to_lowercase();
        if self.accepted.contains(&normalized) {
            return true;
        }
        if self.cannot_inherit.contains(&normalized) {
            return false;
        }
        normalized
            .strip_suffix(".snapshot.json")
            .or_else(|| normalized.strip_suffix(".snapshot"))
            .is_some_and(|stem| {
                self.accepted.contains(&format!("{stem}.md"))
                    || self.accepted.contains(&format!("{stem}.txt"))
            })
    }
}

fn list_files(root: &Path, links: &ProjectLinks) -> Result<Vec<Value>> {
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
        if !links.includes(&name)
            || name.chars().any(char::is_control)
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
        let db = self.db()?;
        let config = super::store::config(&db)?;
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
            let source_id = source["sourceId"]
                .as_str()
                .ok_or("invalid document source")?;
            let links = ProjectLinks::load(&db, source_id, project)?;
            let mut folder = json!({"sourceId":source_id,"root":root,"files":[]});
            match list_files(path, &links) {
                Ok(files) => folder["files"] = json!(files),
                Err(error) => folder["error"] = json!(error),
            }
            folders.push(folder);
        }
        Ok(json!({"folders":folders}))
    }
}
