//! Fidelidade: pontos por compras pagas (OS e vendas de balcão) e validação de resgate.
//! As regras de cálculo são puras (sem banco). O banco entra só em `registrar_compra` e nas consultas.
use rusqlite::{params, Connection, Row};
use serde::Serialize;

use crate::error::{AppError, AppResult, FieldError};

/// Cada R$ 10,00 pagos vale 1 ponto.
pub const CENTAVOS_POR_PONTO: i64 = 1_000;
/// Resgate só em múltiplos de 100 pontos, para não haver fração de ponto perdida.
pub const PONTOS_POR_RESGATE: i64 = 100;
/// Desconto de cada resgate: 100 pontos = R$ 10,00.
pub const CENTAVOS_POR_RESGATE: i64 = 1_000;

#[derive(Debug, Clone, Serialize)]
pub struct Movimento {
    pub id: i64,
    pub cliente_id: i64,
    pub pontos: i64,
    pub origem: String,
    pub referencia: Option<i64>,
    pub criado_em: String,
}

/// Pontos ganhos por um pagamento. Arredonda em centavos e descarta a fração (R$ 19,99 = 1 ponto).
pub fn pontos_da_compra(valor: f64) -> i64 {
    if !valor.is_finite() || valor <= 0.0 {
        return 0;
    }
    let centavos = (valor * 100.0).round() as i64;
    centavos / CENTAVOS_POR_PONTO
}

/// Valida um resgate contra o saldo e devolve o desconto em reais.
pub fn validar_resgate(saldo: i64, pontos: i64) -> AppResult<f64> {
    if pontos <= 0 || pontos % PONTOS_POR_RESGATE != 0 {
        return Err(AppError::Validation(vec![FieldError {
            field: "pontos".into(),
            message: format!("resgate em múltiplos de {PONTOS_POR_RESGATE} pontos"),
        }]));
    }
    if pontos > saldo {
        return Err(AppError::Conflict(format!("Saldo insuficiente: {saldo} pontos disponíveis.")));
    }
    let centavos = pontos / PONTOS_POR_RESGATE * CENTAVOS_POR_RESGATE;
    Ok(centavos as f64 / 100.0)
}

/// Soma dos movimentos: ganhos são positivos, resgates negativos.
pub fn somar(pontos: &[i64]) -> i64 {
    pontos.iter().sum()
}

/// Registra os pontos de uma compra paga. Sem cliente identificado, não acumula.
/// Retorna quantos pontos foram creditados (0 se nada).
pub fn registrar_compra(conn: &Connection, cliente_id: Option<i64>, valor: f64, origem: &str, referencia: i64) -> AppResult<i64> {
    let Some(cliente) = cliente_id else { return Ok(0) };
    let pontos = pontos_da_compra(valor);
    if pontos == 0 {
        return Ok(0);
    }
    conn.execute(
        "INSERT INTO fidelidade_movimentos (cliente_id, pontos, origem, referencia, criado_em) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![cliente, pontos, origem, referencia, chrono::Utc::now().to_rfc3339()],
    )?;
    Ok(pontos)
}

/// Saldo atual do cliente. Erro se o cliente não existe.
pub fn saldo(conn: &Connection, cliente_id: i64) -> AppResult<i64> {
    crate::clientes::buscar(conn, cliente_id)?;
    let pontos: Vec<i64> = conn
        .prepare("SELECT pontos FROM fidelidade_movimentos WHERE cliente_id = ?1")?
        .query_map([cliente_id], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    Ok(somar(&pontos))
}

fn de_linha(r: &Row) -> rusqlite::Result<Movimento> {
    Ok(Movimento {
        id: r.get(0)?,
        cliente_id: r.get(1)?,
        pontos: r.get(2)?,
        origem: r.get(3)?,
        referencia: r.get(4)?,
        criado_em: r.get(5)?,
    })
}

/// Movimentos do cliente, mais recentes primeiro.
pub fn extrato(conn: &Connection, cliente_id: i64) -> AppResult<Vec<Movimento>> {
    crate::clientes::buscar(conn, cliente_id)?;
    let mut stmt = conn.prepare(
        "SELECT id, cliente_id, pontos, origem, referencia, criado_em
         FROM fidelidade_movimentos WHERE cliente_id = ?1 ORDER BY id DESC",
    )?;
    let linhas = stmt.query_map([cliente_id], de_linha)?.collect::<Result<_, _>>()?;
    Ok(linhas)
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn um_ponto_a_cada_dez_reais() {
        assert_eq!(pontos_da_compra(10.0), 1);
        assert_eq!(pontos_da_compra(9.99), 0);
        assert_eq!(pontos_da_compra(19.99), 1);
        assert_eq!(pontos_da_compra(25.0), 2);
        assert_eq!(pontos_da_compra(1000.0), 100);
    }

    #[test]
    fn valores_invalidos_nao_geram_pontos() {
        assert_eq!(pontos_da_compra(0.0), 0);
        assert_eq!(pontos_da_compra(-50.0), 0);
        assert_eq!(pontos_da_compra(f64::NAN), 0);
        assert_eq!(pontos_da_compra(f64::INFINITY), 0);
    }

    #[test]
    fn resgate_de_100_pontos_vale_dez_reais() {
        assert_eq!(validar_resgate(100, 100).unwrap(), 10.0);
        assert_eq!(validar_resgate(250, 200).unwrap(), 20.0);
    }

    #[test]
    fn resgate_nao_deixa_saldo_negativo() {
        assert!(matches!(validar_resgate(99, 100), Err(AppError::Conflict(_))));
        assert!(matches!(validar_resgate(0, 100), Err(AppError::Conflict(_))));
    }

    #[test]
    fn resgate_exige_multiplo_de_100_e_positivo() {
        assert!(matches!(validar_resgate(250, 150), Err(AppError::Validation(_))));
        assert!(matches!(validar_resgate(250, 0), Err(AppError::Validation(_))));
        assert!(matches!(validar_resgate(250, -100), Err(AppError::Validation(_))));
    }

    #[test]
    fn saldo_soma_ganhos_e_resgates() {
        assert_eq!(somar(&[1, 2, 10, -100, 5]), -82);
        assert_eq!(somar(&[]), 0);
    }
}
