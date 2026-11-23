use serde::Serialize;
use thiserror::Error;

pub type AppResult<T> = Result<T, AppError>;

/// Erros de domínio. Serializáveis para o front-end receber a mensagem pronta.
#[derive(Debug, Error, Serialize)]
#[serde(tag = "kind", content = "message", rename_all = "snake_case")]
pub enum AppError {
    #[error("Dados inválidos.")]
    Validation(Vec<FieldError>),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("Erro no banco de dados: {0}")]
    Database(String),
    /// Servidor da rede não respondeu (a operação pode ser guardada para enviar depois).
    #[error("{0}")]
    Offline(String),
}

impl AppError {
    /// Reconstrói um erro recebido de outro computador ({ kind, message }).
    pub fn de_json(v: &serde_json::Value) -> AppError {
        let msg = v.get("message");
        let texto = msg.and_then(|m| m.as_str()).unwrap_or("Erro desconhecido.").to_string();
        match v.get("kind").and_then(|k| k.as_str()).unwrap_or("") {
            "validation" => AppError::Validation(
                serde_json::from_value(msg.cloned().unwrap_or_default()).unwrap_or_default(),
            ),
            "not_found" => AppError::NotFound(texto),
            "conflict" => AppError::Conflict(texto),
            "offline" => AppError::Offline(texto),
            _ => AppError::Database(texto),
        }
    }
}

#[derive(Debug, Clone, Serialize, serde::Deserialize, PartialEq, Eq, Default)]
pub struct FieldError {
    pub field: String,
    pub message: String,
}

// Códigos estendidos do SQLite para traduzir violações de restrição.
const CONSTRAINT_UNIQUE: i32 = 2067;
const CONSTRAINT_FOREIGN_KEY: i32 = 787;
const CONSTRAINT_NOT_NULL: i32 = 1299;

impl From<rusqlite::Error> for AppError {
    fn from(err: rusqlite::Error) -> Self {
        if let rusqlite::Error::SqliteFailure(code, _) = &err {
            match code.extended_code {
                CONSTRAINT_UNIQUE => {
                    return AppError::Conflict("Já existe um registro com esses dados únicos (ex.: placa).".into())
                }
                CONSTRAINT_FOREIGN_KEY => {
                    return AppError::Conflict("Registro não pode ser removido/alterado: está em uso.".into())
                }
                CONSTRAINT_NOT_NULL => {
                    return AppError::Validation(vec![FieldError {
                        field: "*".into(),
                        message: "campo obrigatório ausente".into(),
                    }])
                }
                _ => {}
            }
        }
        AppError::Database(err.to_string())
    }
}
