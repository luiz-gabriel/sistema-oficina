//! Registro de atividades: quem fez o quê e quando. Só o administrador consulta.
//! Fica neste computador (não entra na sincronização), para ser auditoria local.
use rusqlite::{params, Connection};
use serde::Serialize;
use serde_json::Value;

use crate::error::AppResult;

#[derive(Debug, Clone, Serialize)]
pub struct Entrada {
    pub id: i64,
    pub criado_em: String,
    pub usuario: String,
    pub computador: String,
    pub acao: String,
    pub detalhe: String,
}

const LIMITE_DETALHE: usize = 300;
const SEGREDOS: &[&str] = &["senha", "senha_hash", "chave"];

/// Tira dos detalhes qualquer campo que seja segredo (senha, chave de ativação).
pub fn sem_segredos(v: &Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(
            m.iter()
                .map(|(k, val)| {
                    if SEGREDOS.contains(&k.as_str()) {
                        (k.clone(), Value::String("(oculto)".into()))
                    } else {
                        (k.clone(), sem_segredos(val))
                    }
                })
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(sem_segredos).collect()),
        outro => outro.clone(),
    }
}

/// Texto curto do que foi feito (sem segredos e cortado no tamanho máximo).
pub fn resumir(args: &Value) -> String {
    let texto = sem_segredos(args).to_string();
    if texto.chars().count() > LIMITE_DETALHE {
        let corte: String = texto.chars().take(LIMITE_DETALHE).collect();
        format!("{corte}…")
    } else {
        texto
    }
}

pub fn registrar(conn: &Connection, usuario: &str, computador: &str, acao: &str, detalhe: &str) -> AppResult<()> {
    conn.execute(
        "INSERT INTO registro (criado_em, usuario, computador, acao, detalhe) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![chrono::Utc::now().to_rfc3339(), usuario, computador, acao, detalhe],
    )?;
    Ok(())
}

pub fn listar(conn: &Connection, limite: i64) -> AppResult<Vec<Entrada>> {
    let mut stmt = conn.prepare(
        "SELECT id, criado_em, usuario, computador, acao, detalhe FROM registro ORDER BY id DESC LIMIT ?1",
    )?;
    let rows = stmt
        .query_map([limite], |r| {
            Ok(Entrada {
                id: r.get(0)?,
                criado_em: r.get(1)?,
                usuario: r.get(2)?,
                computador: r.get(3)?,
                acao: r.get(4)?,
                detalhe: r.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[cfg(test)]
mod testes {
    use super::*;
    use serde_json::json;

    #[test]
    fn senha_nao_vai_para_o_registro() {
        let v = json!({"id": 3, "dados": {"login": "ana", "senha": "segredo123", "perfil": "funcionario"}});
        let texto = resumir(&v);
        assert!(!texto.contains("segredo123"), "{texto}");
        assert!(texto.contains("(oculto)"));
        assert!(texto.contains("\"login\":\"ana\""));
    }
}
