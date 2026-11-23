use chrono::{Datelike, TimeZone, Utc};
use rusqlite::Connection;
use serde::Serialize;
use std::collections::BTreeMap;

use crate::error::AppResult;

#[derive(Debug, Serialize)]
pub struct EstoqueBaixo {
    pub id: i64,
    pub nome: String,
    pub quantidade: i64,
    pub estoque_minimo: i64,
}

#[derive(Debug, Serialize)]
pub struct Resumo {
    pub clientes: i64,
    pub veiculos: i64,
    pub ordens_por_status: BTreeMap<String, i64>,
    pub faturamento_mes: f64,
    pub estoque_baixo: Vec<EstoqueBaixo>,
}

pub fn resumo(conn: &Connection) -> AppResult<Resumo> {
    let contar = |sql: &str| -> AppResult<i64> { Ok(conn.query_row(sql, [], |r| r.get(0))?) };

    let mut stmt = conn.prepare("SELECT status, COUNT(*) FROM ordens_servico GROUP BY status")?;
    let ordens_por_status = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
        .collect::<Result<_, _>>()?;

    // Início do mês em UTC; datas são gravadas em RFC 3339, então a comparação textual funciona.
    let agora = Utc::now();
    let inicio_mes = Utc
        .with_ymd_and_hms(agora.year(), agora.month(), 1, 0, 0, 0)
        .single()
        .expect("data válida")
        .to_rfc3339();
    let faturamento_mes: f64 = conn.query_row(
        "SELECT COALESCE(SUM(valor_total), 0) FROM ordens_servico
         WHERE status = 'CONCLUIDA' AND data_conclusao >= ?1",
        [inicio_mes],
        |r| r.get(0),
    )?;

    let mut stmt = conn.prepare(
        "SELECT id, nome, quantidade, estoque_minimo FROM itens
         WHERE tipo = 'peca' AND quantidade <= estoque_minimo ORDER BY nome",
    )?;
    let estoque_baixo = stmt
        .query_map([], |r| {
            Ok(EstoqueBaixo { id: r.get(0)?, nome: r.get(1)?, quantidade: r.get(2)?, estoque_minimo: r.get(3)? })
        })?
        .collect::<Result<_, _>>()?;

    Ok(Resumo {
        clientes: contar("SELECT COUNT(*) FROM clientes")?,
        veiculos: contar("SELECT COUNT(*) FROM veiculos")?,
        ordens_por_status,
        faturamento_mes,
        estoque_baixo,
    })
}
