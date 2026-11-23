//! Painel do administrador: séries e totais para os gráficos da tela inicial de gestão. Só leitura.
//!
//! Período com início e fim INCLUSIVOS, em datas AAAA-MM-DD. Valores em dinheiro saem em centavos (inteiros).
//! Datas com horário (ordens, recebimentos) são convertidas para o fuso do computador com `datetime(..., 'localtime')`,
//! então um dia é o dia local, como nos relatórios. Contas a receber e a pagar são a situação de hoje (não dependem do período).
use std::collections::HashMap;

use chrono::{Local, NaiveDate};
use rusqlite::{params, Connection};
use serde::Serialize;

use crate::error::AppResult;
use crate::relatorios::Periodo;

/// Situações de ordem que contam como faturamento (data de conclusão = data em que o serviço foi concluído).
const SITUACOES_FATURADAS: &str = "('CONCLUIDA', 'AGUARDANDO_PAGAMENTO')";
/// Todas as situações de ordem, na ordem em que aparecem no painel.
const SITUACOES_OS: [&str; 5] = ["CRIADA", "EM_ANDAMENTO", "AGUARDANDO_PAGAMENTO", "CONCLUIDA", "CANCELADA"];
/// Formas de recebimento, na ordem do gráfico (sempre as quatro, mesmo com valor zero).
const FORMAS: [&str; 4] = ["dinheiro", "pix", "cartao", "outro"];
const LIMITE_SERVICOS: i64 = 5;
const LIMITE_ESTOQUE: i64 = 10;

/// Faturamento de um dia: ordens e vendas concluídas (ou aguardando pagamento) naquele dia.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DiaFaturamento {
    /// AAAA-MM-DD.
    pub dia: String,
    pub total_centavos: i64,
    pub quantidade: i64,
}

/// Valor efetivamente recebido em uma forma de pagamento no período.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FormaRecebimento {
    pub forma: String,
    pub total_centavos: i64,
}

/// Situação de hoje das contas. `receber_vencido` e `pagar_vencido` são partes de `receber_aberto` e `pagar_aberto`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
pub struct ContasResumo {
    /// Saldo total ainda a receber (inclui o vencido).
    pub receber_aberto: i64,
    /// Parte do saldo a receber cujo vencimento já passou.
    pub receber_vencido: i64,
    /// Saldo total ainda a pagar (inclui o vencido).
    pub pagar_aberto: i64,
    /// Parte do saldo a pagar cujo vencimento já passou.
    pub pagar_vencido: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ServicoTop {
    pub nome: String,
    pub quantidade: i64,
}

/// Peça com saldo igual ou abaixo do mínimo, agora.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct EstoqueBaixo {
    pub nome: String,
    pub quantidade: i64,
    pub estoque_minimo: i64,
}

/// Quantas ordens estão em cada situação atual, entre as que foram abertas no período.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SituacaoOs {
    pub situacao: String,
    pub quantidade: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Painel {
    pub periodo: String,
    pub de: String,
    pub ate: String,
    pub faturamento_por_dia: Vec<DiaFaturamento>,
    pub recebimentos_por_forma: Vec<FormaRecebimento>,
    pub contas: ContasResumo,
    pub servicos_mais_realizados: Vec<ServicoTop>,
    pub estoque_baixo: Vec<EstoqueBaixo>,
    pub os_por_situacao: Vec<SituacaoOs>,
}

fn iso(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

fn centavos(reais: f64) -> i64 {
    (reais * 100.0).round() as i64
}

/// Monta o painel do período `de`..`ate` (AAAA-MM-DD, inclusivos). Recusa período inválido ou maior que 5 anos.
pub fn serie(conn: &Connection, de: &str, ate: &str) -> AppResult<Painel> {
    let p = Periodo::novo(de, ate)?;
    let hoje = Local::now().date_naive();
    Ok(Painel {
        periodo: p.texto(),
        de: iso(p.de),
        ate: iso(p.ate),
        faturamento_por_dia: faturamento_por_dia(conn, &p)?,
        recebimentos_por_forma: recebimentos_por_forma(conn, &p)?,
        contas: contas(conn, hoje)?,
        servicos_mais_realizados: servicos_mais_realizados(conn, &p)?,
        estoque_baixo: estoque_baixo(conn)?,
        os_por_situacao: os_por_situacao(conn, &p)?,
    })
}

/// Um item por dia do período, inclusive os dias sem venda (com zero), para o gráfico não pular dias.
fn faturamento_por_dia(conn: &Connection, p: &Periodo) -> AppResult<Vec<DiaFaturamento>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT date(datetime(data_conclusao, 'localtime')) AS dia, COUNT(*), COALESCE(SUM(valor_total), 0)
         FROM ordens_servico
         WHERE status IN {SITUACOES_FATURADAS} AND data_conclusao IS NOT NULL
           AND date(datetime(data_conclusao, 'localtime')) BETWEEN ?1 AND ?2
         GROUP BY dia",
    ))?;
    let por_dia: HashMap<String, (i64, i64)> = stmt
        .query_map(params![iso(p.de), iso(p.ate)], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, f64>(2)?))
        })?
        .map(|linha| linha.map(|(dia, quantidade, soma)| (dia, (quantidade, centavos(soma)))))
        .collect::<Result<_, _>>()?;

    let mut dias = Vec::new();
    let mut atual = p.de;
    loop {
        let chave = iso(atual);
        let (quantidade, total_centavos) = por_dia.get(&chave).copied().unwrap_or((0, 0));
        dias.push(DiaFaturamento { dia: chave, total_centavos, quantidade });
        if atual >= p.ate {
            break;
        }
        match atual.succ_opt() {
            Some(proximo) => atual = proximo,
            None => break,
        }
    }
    Ok(dias)
}

/// Sempre as quatro formas, na ordem do gráfico, mesmo as que não tiveram recebimento (valor zero).
fn recebimentos_por_forma(conn: &Connection, p: &Periodo) -> AppResult<Vec<FormaRecebimento>> {
    let mut stmt = conn.prepare(
        "SELECT forma, COALESCE(SUM(valor_centavos), 0) FROM recebimentos
         WHERE date(datetime(data, 'localtime')) BETWEEN ?1 AND ?2
         GROUP BY forma",
    )?;
    let por_forma: HashMap<String, i64> = stmt
        .query_map(params![iso(p.de), iso(p.ate)], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
        .collect::<Result<_, _>>()?;
    Ok(FORMAS
        .iter()
        .map(|f| FormaRecebimento { forma: f.to_string(), total_centavos: por_forma.get(*f).copied().unwrap_or(0) })
        .collect())
}

/// Contas a receber e a pagar pelo saldo que ainda falta. Vencida = vencimento anterior a hoje.
fn contas(conn: &Connection, hoje: NaiveDate) -> AppResult<ContasResumo> {
    let hoje_txt = iso(hoje);
    let (receber_aberto, receber_vencido): (i64, i64) = conn.query_row(
        "SELECT COALESCE(SUM(saldo_centavos), 0),
                COALESCE(SUM(CASE WHEN vencimento < ?1 THEN saldo_centavos ELSE 0 END), 0)
         FROM contas_receber WHERE saldo_centavos > 0",
        [&hoje_txt],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    // Mesmo cálculo do relatório de contas: saldo = valor − pagamentos. Compras a prazo também estão em contas_pagar.
    let saldo_pagar = "SELECT cp.id AS chave, cp.vencimento AS venc,
                              cp.valor_centavos - COALESCE((SELECT SUM(p.valor_centavos) FROM pagamentos_pagar p WHERE p.conta_id = cp.id), 0) AS saldo
         FROM contas_pagar cp";
    let (pagar_aberto, pagar_vencido): (i64, i64) = conn.query_row(
        &format!(
            "SELECT COALESCE(SUM(saldo), 0), COALESCE(SUM(CASE WHEN venc < ?1 THEN saldo ELSE 0 END), 0)
             FROM ({saldo_pagar}) WHERE saldo > 0"
        ),
        [&hoje_txt],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok(ContasResumo { receber_aberto, receber_vencido, pagar_aberto, pagar_vencido })
}

/// Os cinco serviços mais realizados (por quantidade; empate por nome). Só serviços de ordens e vendas concluídas no período.
fn servicos_mais_realizados(conn: &Connection, p: &Periodo) -> AppResult<Vec<ServicoTop>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT i.nome, SUM(oi.quantidade) AS qtd FROM ordem_itens oi
         JOIN itens i ON i.id = oi.item_id
         JOIN ordens_servico os ON os.id = oi.os_id
         WHERE i.tipo = 'servico' AND os.status IN {SITUACOES_FATURADAS} AND os.data_conclusao IS NOT NULL
           AND date(datetime(os.data_conclusao, 'localtime')) BETWEEN ?1 AND ?2
         GROUP BY i.id, i.nome
         ORDER BY qtd DESC, i.nome
         LIMIT {LIMITE_SERVICOS}",
    ))?;
    let linhas = stmt
        .query_map(params![iso(p.de), iso(p.ate)], |r| Ok(ServicoTop { nome: r.get(0)?, quantidade: r.get(1)? }))?
        .collect::<Result<_, _>>()?;
    Ok(linhas)
}

/// Peças com saldo igual ou abaixo do mínimo, agora (serviços não têm estoque e ficam de fora).
fn estoque_baixo(conn: &Connection) -> AppResult<Vec<EstoqueBaixo>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT nome, quantidade, estoque_minimo FROM itens
         WHERE tipo = 'peca' AND quantidade <= estoque_minimo
         ORDER BY quantidade, nome
         LIMIT {LIMITE_ESTOQUE}",
    ))?;
    let linhas = stmt
        .query_map([], |r| Ok(EstoqueBaixo { nome: r.get(0)?, quantidade: r.get(1)?, estoque_minimo: r.get(2)? }))?
        .collect::<Result<_, _>>()?;
    Ok(linhas)
}

/// Situação atual das ordens abertas no período (pela data de entrada). Sempre as cinco situações, na ordem do painel.
fn os_por_situacao(conn: &Connection, p: &Periodo) -> AppResult<Vec<SituacaoOs>> {
    let mut stmt = conn.prepare(
        "SELECT status, COUNT(*) FROM ordens_servico
         WHERE date(datetime(data_entrada, 'localtime')) BETWEEN ?1 AND ?2
         GROUP BY status",
    )?;
    let por_status: HashMap<String, i64> = stmt
        .query_map(params![iso(p.de), iso(p.ate)], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
        .collect::<Result<_, _>>()?;
    Ok(SITUACOES_OS
        .iter()
        .map(|s| SituacaoOs { situacao: s.to_string(), quantidade: por_status.get(*s).copied().unwrap_or(0) })
        .collect())
}
