// 문서 설정과 출처의 엄격한 입력 계약을 정의한다.
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DocumentConfigView {
    pub revision: u64,
    pub nas_interval_sec: u64,
    pub local_interval_sec: u64,
    pub sources: Vec<DocumentSource>,
}
impl Default for DocumentConfigView {
    fn default() -> Self {
        Self {
            revision: 0,
            nas_interval_sec: 30,
            local_interval_sec: 5,
            sources: vec![],
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DocumentSource {
    pub source_id: String,
    pub scope: String,
    pub project_id: Option<String>,
    pub location: String,
    pub root: String,
    pub roles: Vec<String>,
    pub enabled: bool,
    #[serde(default)]
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default)]
    pub archived: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceSpan {
    pub version_id: String,
    pub sha256: String,
    pub locator: Value,
}
