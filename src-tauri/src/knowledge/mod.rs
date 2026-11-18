// 프로젝트 문서와 진행 근거를 기존 엔진에 연결한다.
pub mod models;
pub mod scanner;
pub(crate) mod store;
use crate::service::Engine;
use rusqlite::Connection;
use serde_json::Value;
pub type Result<T> = std::result::Result<T, String>;
pub(crate) fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
pub(crate) fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}
#[derive(Clone)]
pub struct KnowledgeService {
    pub(crate) engine: Engine,
}
impl KnowledgeService {
    pub(crate) fn new(engine: Engine) -> Self {
        Self { engine }
    }
    pub(crate) fn db(&self) -> Result<Connection> {
        self.engine.db()
    }
    pub(crate) fn require_project(&self, id: &str) -> Result<()> {
        if !self.engine.registry()?["projects"]
            .as_array()
            .is_some_and(|ps| ps.iter().any(|p| p["id"] == id && p["archived"] != true))
        {
            return Err("project unavailable".into());
        }
        Ok(())
    }
    pub fn get_document_config(&self) -> Result<Value> {
        store::config(&self.db()?)
    }
}
