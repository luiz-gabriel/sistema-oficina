//! Caixa: abertura com troco inicial, recebimentos (ligados ao caixa aberto), sangrias e suprimentos,
//! e fechamento com conferência do dinheiro da gaveta.
//!
//! Dinheiro esperado na gaveta = troco inicial + recebimentos em dinheiro + suprimentos - sangrias.
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::Serialize;

use crate::error::{AppError, AppResult, FieldError};

/// Formas de pagamento aceitas ao concluir uma ordem.
pub const PAGAMENTOS: &[&str] = &["dinheiro", "pix", "cartao", "outro"];

#[derive(Debug, Clone, Serialize)]
pub struct Sessao {
    pub id: i64,
    pub aberto_em: String,
    pub aberto_por: String,
    pub valor_abertura: f64,
    pub fechado_em: Option<String>,
    pub fechado_por: Option<String>,
    pub valor_contado: Option<f64>,
    pub dinheiro_esperado: Option<f64>,
    pub diferenca: Option<f64>,
    pub observacao: Option<String>,
}

/// Quanto entrou em cada forma de pagamento desde a abertura da sessão.
#[derive(Debug, Clone, Serialize, Default)]
pub struct Recebimentos {
    pub dinheiro: f64,
    pub pix: f64,
    pub cartao: f64,
    pub outro: f64,
    pub total: f64,
    pub atendimentos: i64,
}

/// Sangria (retirada de dinheiro da gaveta) ou suprimento (entrada de dinheiro) feito com o caixa aberto.
#[derive(Debug, Clone, Serialize)]
pub struct Movimento {
    pub id: i64,
    pub caixa_id: i64,
    pub tipo: String,
    pub valor: f64,
    pub motivo: String,
    pub usuario: String,
    pub criado_em: String,
}

/// Uma ordem concluída (recebimento) dentro do período do caixa.
#[derive(Debug, Clone, Serialize)]
pub struct Recebido {
    /// Número da venda ou da OS (ordem_id).
    pub os_id: i64,
    /// Valor recebido NESTE caixa para esta venda (soma dos pagamentos desta venda neste caixa).
    pub valor: f64,
    /// Formas usadas neste caixa, separadas por vírgula (ex.: "dinheiro, pix").
    pub pagamento: String,
    pub concluido_em: String,
    /// "OS" ou "BALCAO" (venda de balcão).
    pub tipo: String,
    pub cliente: String,
    /// Valor total da venda ou OS (o que foi cobrado, não só o que entrou neste caixa).
    pub total_venda: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Situacao {
    pub aberto: bool,
    pub sessao: Option<Sessao>,
    pub recebimentos: Recebimentos,
    pub sangrias: f64,
    pub suprimentos: f64,
    /// Dinheiro que deveria estar na gaveta: abertura + recebimentos em dinheiro + suprimentos - sangrias.
    pub dinheiro_esperado: f64,
    pub movimentos: Vec<Movimento>,
    /// True quando o caixa aberto é de outro operador: a tela mostra só quem abriu e quando.
    #[serde(default)]
    pub de_outro_operador: bool,
}

/// Resumo completo de um caixa (aberto ou fechado), usado na tela e no cupom de fechamento.
#[derive(Debug, Clone, Serialize)]
pub struct Relatorio {
    pub sessao: Sessao,
    pub recebimentos: Recebimentos,
    pub sangrias: f64,
    pub suprimentos: f64,
    pub dinheiro_esperado: f64,
    pub dinheiro_contado: Option<f64>,
    pub diferenca: Option<f64>,
    pub recebidos: Vec<Recebido>,
    pub movimentos: Vec<Movimento>,
}

const COLUNAS: &str = "id, aberto_em, aberto_por, valor_abertura, fechado_em, fechado_por, valor_contado, dinheiro_esperado, diferenca, observacao";

fn de_linha(r: &Row) -> rusqlite::Result<Sessao> {
    Ok(Sessao {
        id: r.get(0)?,
        aberto_em: r.get(1)?,
        aberto_por: r.get(2)?,
        valor_abertura: r.get(3)?,
        fechado_em: r.get(4)?,
        fechado_por: r.get(5)?,
        valor_contado: r.get(6)?,
        dinheiro_esperado: r.get(7)?,
        diferenca: r.get(8)?,
        observacao: r.get(9)?,
    })
}

fn de_movimento(r: &Row) -> rusqlite::Result<Movimento> {
    Ok(Movimento {
        id: r.get(0)?,
        caixa_id: r.get(1)?,
        tipo: r.get(2)?,
        valor: r.get(3)?,
        motivo: r.get(4)?,
        usuario: r.get(5)?,
        criado_em: r.get(6)?,
    })
}

fn erro(campo: &str, msg: &str) -> AppError {
    AppError::Validation(vec![FieldError { field: campo.into(), message: msg.into() }])
}

fn centavos(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// Valor em dinheiro digitado: finito e não negativo, arredondado a centavos.
fn valor(v: Option<f64>, campo: &str) -> AppResult<f64> {
    match v {
        Some(x) if x.is_finite() && x >= 0.0 => Ok(centavos(x)),
        _ => Err(erro(campo, "informe um valor igual ou maior que zero")),
    }
}

/// Dinheiro que deve estar na gaveta, a partir das parcelas do caixa.
fn esperado_em_dinheiro(abertura: f64, dinheiro_recebido: f64, sangrias: f64, suprimentos: f64) -> f64 {
    centavos(abertura + dinheiro_recebido + suprimentos - sangrias)
}

fn sessao_aberta(conn: &Connection) -> AppResult<Option<Sessao>> {
    Ok(conn
        .query_row(&format!("SELECT {COLUNAS} FROM caixas WHERE fechado_em IS NULL ORDER BY id DESC LIMIT 1"), [], de_linha)
        .optional()?)
}

fn sessao_por_id(conn: &Connection, id: i64) -> AppResult<Option<Sessao>> {
    Ok(conn
        .query_row(&format!("SELECT {COLUNAS} FROM caixas WHERE id = ?1"), [id], de_linha)
        .optional()?)
}

/// Recebimentos gravados neste caixa, somados por forma. O caixa soma recebimentos, nunca o valor das OS.
fn recebimentos(conn: &Connection, caixa_id: i64) -> AppResult<Recebimentos> {
    let mut r = Recebimentos::default();
    let mut stmt = conn.prepare("SELECT forma, COALESCE(SUM(valor_centavos), 0) FROM recebimentos WHERE caixa_id = ?1 GROUP BY forma")?;
    let linhas = stmt.query_map([caixa_id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))?;
    for linha in linhas {
        let (forma, centavos) = linha?;
        let total = centavos as f64 / 100.0;
        match forma.as_str() {
            "dinheiro" => r.dinheiro += total,
            "pix" => r.pix += total,
            "cartao" => r.cartao += total,
            _ => r.outro += total,
        }
        r.total += total;
    }
    r.atendimentos = conn.query_row(
        "SELECT COUNT(DISTINCT ordem_id) FROM recebimentos WHERE caixa_id = ?1",
        [caixa_id],
        |row| row.get(0),
    )?;
    Ok(r)
}

/// Uma linha por venda ou OS recebida neste caixa, com cliente, total cobrado e o que entrou aqui.
fn recebidos(conn: &Connection, caixa_id: i64) -> AppResult<Vec<Recebido>> {
    let mut stmt = conn.prepare(
        "SELECT r.ordem_id, COALESCE(o.tipo, 'OS'), COALESCE(c.nome, '—'), COALESCE(o.valor_total, 0),
                SUM(r.valor_centavos), GROUP_CONCAT(DISTINCT r.forma), MAX(r.data)
         FROM recebimentos r
         JOIN ordens_servico o ON o.id = r.ordem_id
         LEFT JOIN veiculos v ON v.id = o.veiculo_id
         LEFT JOIN clientes c ON c.id = COALESCE(o.cliente_id, v.cliente_id)
         WHERE r.caixa_id = ?1
         GROUP BY r.ordem_id
         ORDER BY MAX(r.data), r.ordem_id",
    )?;
    let linhas = stmt
        .query_map([caixa_id], |row| {
            let recebido_centavos: i64 = row.get(4)?;
            let total: f64 = row.get(3)?;
            let formas: String = row.get::<_, Option<String>>(5)?.unwrap_or_default();
            Ok(Recebido {
                os_id: row.get(0)?,
                tipo: row.get(1)?,
                cliente: row.get(2)?,
                total_venda: centavos(total),
                valor: recebido_centavos as f64 / 100.0,
                pagamento: formas.split(',').map(str::trim).collect::<Vec<_>>().join(", "),
                concluido_em: row.get::<_, Option<String>>(6)?.unwrap_or_default(),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(linhas)
}

/// Movimentos do caixa: (total de sangrias, total de suprimentos).
fn totais_movimentos(conn: &Connection, caixa_id: i64) -> AppResult<(f64, f64)> {
    let mut stmt = conn.prepare("SELECT tipo, COALESCE(SUM(valor), 0) FROM caixa_movimentos WHERE caixa_id = ?1 GROUP BY tipo")?;
    let mut sangrias = 0.0;
    let mut suprimentos = 0.0;
    for linha in stmt.query_map([caixa_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?)))? {
        let (tipo, total) = linha?;
        match tipo.as_str() {
            "sangria" => sangrias += total,
            _ => suprimentos += total,
        }
    }
    Ok((centavos(sangrias), centavos(suprimentos)))
}

fn movimentos_do_caixa(conn: &Connection, caixa_id: i64) -> AppResult<Vec<Movimento>> {
    let mut stmt = conn.prepare(
        "SELECT id, caixa_id, tipo, valor, motivo, usuario, criado_em FROM caixa_movimentos WHERE caixa_id = ?1 ORDER BY id",
    )?;
    let linhas = stmt.query_map([caixa_id], de_movimento)?.collect::<Result<Vec<_>, _>>()?;
    Ok(linhas)
}

/// Dinheiro que está na gaveta agora (com o caixa aberto).
fn dinheiro_disponivel(conn: &Connection, sessao: &Sessao) -> AppResult<f64> {
    let rec = recebimentos(conn, sessao.id)?;
    let (sangrias, suprimentos) = totais_movimentos(conn, sessao.id)?;
    Ok(esperado_em_dinheiro(sessao.valor_abertura, rec.dinheiro, sangrias, suprimentos))
}

/// Situação atual: se há caixa aberto, quanto entrou nele e quanto deve haver em dinheiro.
pub fn situacao(conn: &Connection) -> AppResult<Situacao> {
    match sessao_aberta(conn)? {
        Some(s) => {
            let rec = recebimentos(conn, s.id)?;
            let (sangrias, suprimentos) = totais_movimentos(conn, s.id)?;
            let esperado = esperado_em_dinheiro(s.valor_abertura, rec.dinheiro, sangrias, suprimentos);
            let movimentos = movimentos_do_caixa(conn, s.id)?;
            Ok(Situacao {
                aberto: true,
                dinheiro_esperado: esperado,
                recebimentos: rec,
                sangrias,
                suprimentos,
                movimentos,
                de_outro_operador: false,
                sessao: Some(s),
            })
        }
        None => Ok(Situacao {
            aberto: false,
            sessao: None,
            recebimentos: Recebimentos::default(),
            sangrias: 0.0,
            suprimentos: 0.0,
            dinheiro_esperado: 0.0,
            movimentos: Vec::new(),
            de_outro_operador: false,
        }),
    }
}

/// Abre o caixa com o dinheiro inicial (troco). Só pode haver um caixa aberto por vez.
pub fn abrir(conn: &Connection, valor_inicial: Option<f64>, usuario: &str, agora: &str) -> AppResult<Sessao> {
    if let Some(aberto) = sessao_aberta(conn)? {
        return Err(AppError::Conflict(format!(
            "O caixa já está aberto por {} desde {}. Ele precisa fechar o caixa, ou o administrador fecha.",
            aberto.aberto_por, aberto.aberto_em
        )));
    }
    let inicial = valor(valor_inicial.or(Some(0.0)), "valor_inicial")?;
    conn.execute(
        "INSERT INTO caixas (aberto_em, aberto_por, valor_abertura) VALUES (?1, ?2, ?3)",
        params![agora, usuario, inicial],
    )?;
    let id = conn.last_insert_rowid();
    Ok(conn.query_row(&format!("SELECT {COLUNAS} FROM caixas WHERE id = ?1"), [id], de_linha)?)
}

/// Sangria (retirada de dinheiro) ou suprimento (entrada de dinheiro) no caixa aberto.
/// Sangria maior que o dinheiro disponível na gaveta é recusada.
pub fn registrar_movimento(
    conn: &Connection,
    tipo: &str,
    valor_informado: Option<f64>,
    motivo: Option<String>,
    usuario: &str,
    agora: &str,
) -> AppResult<Movimento> {
    let sessao = sessao_aberta(conn)?
        .ok_or_else(|| AppError::Conflict("Abra o caixa antes de registrar sangria ou suprimento.".into()))?;
    let v = valor(valor_informado, "valor")?;
    if v <= 0.0 {
        return Err(erro("valor", "informe um valor maior que zero"));
    }
    let motivo = motivo
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty())
        .ok_or_else(|| erro("motivo", "informe o motivo"))?;
    if tipo == "sangria" {
        let disponivel = dinheiro_disponivel(conn, &sessao)?;
        if v > disponivel {
            return Err(erro(
                "valor",
                &format!("não há dinheiro suficiente na gaveta (disponível: R$ {disponivel:.2})").replace('.', ","),
            ));
        }
    }
    conn.execute(
        "INSERT INTO caixa_movimentos (caixa_id, tipo, valor, motivo, usuario, criado_em) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![sessao.id, tipo, v, motivo, usuario, agora],
    )?;
    let id = conn.last_insert_rowid();
    Ok(conn.query_row(
        "SELECT id, caixa_id, tipo, valor, motivo, usuario, criado_em FROM caixa_movimentos WHERE id = ?1",
        [id],
        de_movimento,
    )?)
}

/// Fecha o caixa. `valor_contado` é o dinheiro conferido na gaveta. A diferença mostra sobra ou falta.
pub fn fechar(
    conn: &Connection,
    id: i64,
    valor_contado: Option<f64>,
    observacao: Option<String>,
    usuario: &str,
    agora: &str,
) -> AppResult<Sessao> {
    let sessao = sessao_aberta(conn)?
        .filter(|s| s.id == id)
        .ok_or_else(|| AppError::Conflict("Não há caixa aberto para fechar.".into()))?;
    let contado = valor(valor_contado, "valor_contado")?;
    let rec = recebimentos(conn, sessao.id)?;
    let (sangrias, suprimentos) = totais_movimentos(conn, sessao.id)?;
    let esperado = esperado_em_dinheiro(sessao.valor_abertura, rec.dinheiro, sangrias, suprimentos);
    let diferenca = centavos(contado - esperado);
    let obs = observacao.map(|o| o.trim().to_string()).filter(|o| !o.is_empty());
    let alterados = conn.execute(
        "UPDATE caixas SET fechado_em = ?1, fechado_por = ?2, valor_contado = ?3, dinheiro_esperado = ?4, diferenca = ?5, observacao = ?6
         WHERE id = ?7 AND fechado_em IS NULL",
        params![agora, usuario, contado, esperado, diferenca, obs, sessao.id],
    )?;
    if alterados == 0 {
        // Outro fechamento chegou primeiro.
        return Err(AppError::Conflict("Este caixa já foi fechado.".into()));
    }
    Ok(conn.query_row(&format!("SELECT {COLUNAS} FROM caixas WHERE id = ?1"), [sessao.id], de_linha)?)
}

/// Resumo completo de um caixa (aberto ou fechado): recebimentos, movimentos, ordens recebidas e conferência.
pub fn relatorio(conn: &Connection, id: i64) -> AppResult<Relatorio> {
    let sessao = sessao_por_id(conn, id)?.ok_or_else(|| AppError::NotFound("Caixa não encontrado.".into()))?;
    let rec = recebimentos(conn, sessao.id)?;
    let (sangrias, suprimentos) = totais_movimentos(conn, sessao.id)?;
    Ok(Relatorio {
        dinheiro_esperado: esperado_em_dinheiro(sessao.valor_abertura, rec.dinheiro, sangrias, suprimentos),
        dinheiro_contado: sessao.valor_contado,
        diferenca: sessao.diferenca,
        recebidos: recebidos(conn, sessao.id)?,
        movimentos: movimentos_do_caixa(conn, sessao.id)?,
        recebimentos: rec,
        sangrias,
        suprimentos,
        sessao,
    })
}

/// Últimos caixas fechados, para conferência.
pub fn historico(conn: &Connection, limite: i64) -> AppResult<Vec<Sessao>> {
    let mut stmt = conn.prepare(&format!("SELECT {COLUNAS} FROM caixas ORDER BY id DESC LIMIT ?1"))?;
    let rows = stmt.query_map([limite], de_linha)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Só recebe pagamento com caixa aberto. Usado ao concluir uma ordem.
pub fn exigir_aberto(conn: &Connection) -> AppResult<()> {
    if sessao_aberta(conn)?.is_none() {
        return Err(AppError::Conflict("Abra o caixa antes de receber o pagamento.".into()));
    }
    Ok(())
}
