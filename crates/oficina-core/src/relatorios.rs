//! Relatórios de gestão. Só leitura: nenhuma função aqui grava no banco.
//!
//! Valores em dinheiro saem em centavos (inteiros). O período tem início e fim INCLUSIVOS, em datas AAAA-MM-DD,
//! no fuso horário do computador. Datas com horário (ordens, recebimentos, despesas) são comparadas pelo instante
//! real (`datetime()` do SQLite converte o fuso); datas simples (vencimentos, datas de compra) pelo próprio dia.
use chrono::{Local, NaiveDate};
use rusqlite::{params, Connection};
use serde::Serialize;

use crate::error::{AppError, AppResult, FieldError};

/// Período máximo de consulta: 5 anos.
const DIAS_MAXIMOS: i64 = 366 * 5;

/// Período de datas, com início e fim inclusivos.
#[derive(Debug, Clone, Copy)]
pub struct Periodo {
    pub de: NaiveDate,
    pub ate: NaiveDate,
}

fn erro_campo(campo: &str, mensagem: &str) -> AppError {
    AppError::Validation(vec![FieldError { field: campo.into(), message: mensagem.into() }])
}

fn data_br(d: NaiveDate) -> String {
    d.format("%d/%m/%Y").to_string()
}

fn iso(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

impl Periodo {
    /// Aceita datas AAAA-MM-DD. Recusa período invertido ou maior que 5 anos.
    pub fn novo(de: &str, ate: &str) -> AppResult<Periodo> {
        let de = NaiveDate::parse_from_str(de.trim(), "%Y-%m-%d").map_err(|_| erro_campo("de", "data inicial inválida"))?;
        let ate = NaiveDate::parse_from_str(ate.trim(), "%Y-%m-%d").map_err(|_| erro_campo("ate", "data final inválida"))?;
        if ate < de {
            return Err(erro_campo("ate", "a data final não pode ser antes da data inicial"));
        }
        if (ate - de).num_days() > DIAS_MAXIMOS {
            return Err(erro_campo("ate", "o período não pode passar de 5 anos"));
        }
        Ok(Periodo { de, ate })
    }

    pub fn texto(&self) -> String {
        format!("{} a {}", data_br(self.de), data_br(self.ate))
    }

    /// Parâmetros (início, fim exclusivo) para datas com horário, comparadas com `datetime()`.
    /// Meia-noite local de `de` e de `ate + 1`, em formato 'AAAA-MM-DD HH:MM:SS' UTC.
    fn limites_instante(&self) -> (String, String) {
        (instante(self.de), instante(self.ate.succ_opt().unwrap_or(self.ate)))
    }

    /// Datas simples (AAAA-MM-DD), inclusivas.
    fn datas(&self) -> (String, String) {
        (iso(self.de), iso(self.ate))
    }
}

/// Meia-noite local de um dia, como 'AAAA-MM-DD HH:MM:SS' em UTC (formato que `datetime()` devolve).
fn instante(dia: NaiveDate) -> String {
    use chrono::{TimeZone, Utc};
    let meia_noite = dia.and_hms_opt(0, 0, 0).expect("meia-noite válida");
    let utc = Local
        .from_local_datetime(&meia_noite)
        .earliest()
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or_else(|| Utc.from_utc_datetime(&meia_noite));
    utc.format("%Y-%m-%d %H:%M:%S").to_string()
}

/// Quantidade e total em centavos.
#[derive(Debug, Clone, Copy, Serialize, Default, PartialEq, Eq)]
pub struct Total {
    pub quantidade: i64,
    pub centavos: i64,
}

fn centavos(reais: f64) -> i64 {
    (reais * 100.0).round() as i64
}

// ---------- Faturamento ----------

#[derive(Debug, Clone, Serialize)]
pub struct Faturamento {
    pub periodo: String,
    /// Ordens de serviço (com veículo) concluídas.
    pub servicos_os: Total,
    /// Vendas de balcão (sem veículo) concluídas.
    pub vendas_balcao: Total,
    pub total: Total,
}

pub fn faturamento(conn: &Connection, p: &Periodo) -> AppResult<Faturamento> {
    let (ini, fim) = p.limites_instante();
    let mut f = Faturamento { periodo: p.texto(), servicos_os: Total::default(), vendas_balcao: Total::default(), total: Total::default() };
    let mut stmt = conn.prepare(
        "SELECT tipo, COUNT(*), COALESCE(SUM(valor_total), 0) FROM ordens_servico
         WHERE status = 'CONCLUIDA' AND datetime(data_conclusao) >= ?1 AND datetime(data_conclusao) < ?2 GROUP BY tipo",
    )?;
    let linhas = stmt.query_map(params![ini, fim], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, f64>(2)?)))?;
    for linha in linhas {
        let (tipo, quantidade, soma) = linha?;
        let t = Total { quantidade, centavos: centavos(soma) };
        if tipo == "BALCAO" {
            f.vendas_balcao = t;
        } else {
            f.servicos_os = t;
        }
    }
    f.total = Total {
        quantidade: f.servicos_os.quantidade + f.vendas_balcao.quantidade,
        centavos: f.servicos_os.centavos + f.vendas_balcao.centavos,
    };
    Ok(f)
}

// ---------- Recebimentos ----------

/// Valores efetivamente recebidos (tabela `recebimentos`), por forma de pagamento.
#[derive(Debug, Clone, Serialize)]
pub struct Recebimentos {
    pub periodo: String,
    pub dinheiro: Total,
    pub pix: Total,
    pub cartao: Total,
    pub outro: Total,
    pub total: Total,
}

pub fn recebimentos(conn: &Connection, p: &Periodo) -> AppResult<Recebimentos> {
    let (ini, fim) = p.limites_instante();
    let mut r = Recebimentos {
        periodo: p.texto(),
        dinheiro: Total::default(),
        pix: Total::default(),
        cartao: Total::default(),
        outro: Total::default(),
        total: Total::default(),
    };
    let mut stmt = conn.prepare(
        "SELECT forma, COUNT(*), COALESCE(SUM(valor_centavos), 0) FROM recebimentos
         WHERE datetime(data) >= ?1 AND datetime(data) < ?2 GROUP BY forma",
    )?;
    let linhas = stmt.query_map(params![ini, fim], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?)))?;
    for linha in linhas {
        let (forma, quantidade, cents) = linha?;
        let t = Total { quantidade, centavos: cents };
        match forma.as_str() {
            "dinheiro" => r.dinheiro = t,
            "pix" => r.pix = t,
            "cartao" => r.cartao = t,
            _ => r.outro = Total { quantidade: r.outro.quantidade + t.quantidade, centavos: r.outro.centavos + t.centavos },
        }
    }
    let todas = [r.dinheiro, r.pix, r.cartao, r.outro];
    r.total = Total { quantidade: todas.iter().map(|t| t.quantidade).sum(), centavos: todas.iter().map(|t| t.centavos).sum() };
    Ok(r)
}

// ---------- Contas em aberto e vencidas ----------

#[derive(Debug, Clone, Serialize)]
pub struct Contas {
    pub hoje: String,
    /// Ordens criadas ou em andamento, hoje (não depende do período).
    pub ordens_em_aberto: Total,
    /// Ordens em aberto cujo prazo previsto já passou.
    pub ordens_com_prazo_vencido: Total,
    /// Saldo ainda a receber (contas a receber).
    pub contas_a_receber: Total,
    pub contas_a_receber_vencidas: Total,
    /// Saldo ainda a pagar (contas a pagar, já descontados os pagamentos).
    pub contas_a_pagar: Total,
    pub contas_a_pagar_vencidas: Total,
    pub observacao: String,
}

pub fn contas(conn: &Connection, hoje: NaiveDate) -> AppResult<Contas> {
    let hoje_txt = iso(hoje);
    let (q, c): (i64, f64) = conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(valor_total), 0) FROM ordens_servico WHERE status IN ('CRIADA', 'EM_ANDAMENTO')",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let (qv, cv): (i64, f64) = conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(os.valor_total), 0) FROM ordens_servico os
         JOIN ordem_km k ON k.os_id = os.id
         WHERE os.status IN ('CRIADA', 'EM_ANDAMENTO') AND k.prazo_previsto IS NOT NULL AND k.prazo_previsto < ?1",
        [&hoje_txt],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let (ar_q, ar_c, arv_q, arv_c): (i64, i64, i64, i64) = conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(saldo_centavos), 0),
                COALESCE(SUM(CASE WHEN vencimento < ?1 THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN vencimento < ?1 THEN saldo_centavos ELSE 0 END), 0)
         FROM contas_receber WHERE saldo_centavos > 0",
        [&hoje_txt],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )?;
    // Contas a pagar: saldo = valor − pagamentos. Compras a prazo também estão em contas_pagar (sem UNION, para não contar duas vezes).
    let saldo_pagar = "SELECT cp.id AS chave, cp.vencimento AS venc, cp.valor_centavos - COALESCE((SELECT SUM(p.valor_centavos) FROM pagamentos_pagar p WHERE p.conta_id = cp.id), 0) AS saldo
         FROM contas_pagar cp";
    let (ap_q, ap_c, apv_q, apv_c): (i64, i64, i64, i64) = conn.query_row(
        &format!(
            "SELECT COUNT(*), COALESCE(SUM(saldo), 0),
                    COALESCE(SUM(CASE WHEN venc < ?1 THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN venc < ?1 THEN saldo ELSE 0 END), 0)
             FROM ({saldo_pagar}) WHERE saldo > 0"
        ),
        [&hoje_txt],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )?;
    Ok(Contas {
        hoje: data_br(hoje),
        ordens_em_aberto: Total { quantidade: q, centavos: centavos(c) },
        ordens_com_prazo_vencido: Total { quantidade: qv, centavos: centavos(cv) },
        contas_a_receber: Total { quantidade: ar_q, centavos: ar_c },
        contas_a_receber_vencidas: Total { quantidade: arv_q, centavos: arv_c },
        contas_a_pagar: Total { quantidade: ap_q, centavos: ap_c },
        contas_a_pagar_vencidas: Total { quantidade: apv_q, centavos: apv_c },
        observacao: "Contas a receber e a pagar são as do módulo financeiro, pelo saldo que ainda falta. \
                      Vencidas são as de vencimento anterior a hoje. Em aberto nas ordens são as ainda não concluídas."
            .into(),
    })
}

// ---------- Lucro ----------

#[derive(Debug, Clone, Serialize)]
pub struct Lucro {
    pub periodo: String,
    pub receita_centavos: i64,
    /// Soma de quantidade × custo unitário gravado na saída de cada peça. Linhas antigas, sem custo gravado,
    /// usam a média atual das compras do item (ou o preço de custo cadastrado, se não houve compra).
    pub custo_pecas_centavos: i64,
    /// Despesas lançadas no período (contas a pagar que não são de compra de peças).
    pub despesas_centavos: i64,
    pub resultado_centavos: i64,
    /// O que entra nos números acima.
    pub incluido: Vec<String>,
    /// O que NÃO entra nos números acima.
    pub nao_incluido: Vec<String>,
}

pub fn lucro(conn: &Connection, p: &Periodo) -> AppResult<Lucro> {
    let (ini, fim) = p.limites_instante();
    let receita = faturamento(conn, p)?.total.centavos;

    // Custo da saída: o custo gravado na baixa (custo_unitario_centavos), que não muda com compras posteriores.
    // Fallback só para linhas antigas sem custo gravado: média atual das compras do item (total pago ÷ quantidade
    // comprada) ou, sem compra, o preço de custo do cadastro. Essa média pode incluir compras feitas depois da venda.
    let mut stmt = conn.prepare(
        "SELECT oi.quantidade, oi.custo_unitario_centavos,
                COALESCE((SELECT CAST(SUM(ci.subtotal_centavos) AS REAL) / SUM(ci.quantidade)
                            FROM compra_itens ci WHERE ci.item_id = i.id), i.preco_custo * 100.0)
         FROM ordem_itens oi
         JOIN itens i ON i.id = oi.item_id
         JOIN ordens_servico os ON os.id = oi.os_id
         WHERE i.tipo = 'peca' AND os.status = 'CONCLUIDA'
           AND datetime(os.data_conclusao) >= ?1 AND datetime(os.data_conclusao) < ?2",
    )?;
    let custo: i64 = stmt
        .query_map(params![ini, fim], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, Option<i64>>(1)?, r.get::<_, f64>(2)?))
        })?
        .map(|linha| {
            linha.map(|(q, gravado, media)| match gravado {
                Some(custo_unitario) => q * custo_unitario,
                None => (q as f64 * media).round() as i64,
            })
        })
        .sum::<Result<i64, _>>()?;

    let despesas: i64 = conn.query_row(
        "SELECT COALESCE(SUM(valor_centavos), 0) FROM contas_pagar
         WHERE compra_id IS NULL AND datetime(data) >= ?1 AND datetime(data) < ?2",
        params![ini, fim],
        |r| r.get(0),
    )?;

    Ok(Lucro {
        periodo: p.texto(),
        receita_centavos: receita,
        custo_pecas_centavos: custo,
        despesas_centavos: despesas,
        resultado_centavos: receita - custo - despesas,
        incluido: vec![
            "Receita: valor das ordens de serviço e das vendas de balcão concluídas no período.".into(),
            "Custo das peças vendidas: quantidade × custo da peça no momento da venda. Vendas antigas, sem esse custo gravado, usam a média atual das compras (ou o preço de custo cadastrado).".into(),
            "Despesas: contas lançadas no período que não são compra de peças (aluguel, luz, salários etc.).".into(),
        ],
        nao_incluido: vec![
            "Mão de obra: os serviços não têm custo registrado. Entram só como receita.".into(),
            "Compras de peças: não são despesa do período; o custo entra quando a peça é vendida.".into(),
            "Ordens ainda não concluídas: só entram quando são concluídas no período.".into(),
        ],
    })
}

// ---------- Serviços ----------

#[derive(Debug, Clone, Serialize)]
pub struct ServicoRealizado {
    pub item_id: i64,
    pub nome: String,
    pub quantidade: i64,
    pub centavos: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Servicos {
    pub periodo: String,
    pub quantidade: i64,
    pub centavos: i64,
    /// Os 10 serviços mais realizados (por quantidade, depois por valor).
    pub mais_realizados: Vec<ServicoRealizado>,
}

pub fn servicos(conn: &Connection, p: &Periodo) -> AppResult<Servicos> {
    let (ini, fim) = p.limites_instante();
    let mut stmt = conn.prepare(
        "SELECT i.id, i.nome, SUM(oi.quantidade), SUM(oi.subtotal) FROM ordem_itens oi
         JOIN itens i ON i.id = oi.item_id
         JOIN ordens_servico os ON os.id = oi.os_id
         WHERE i.tipo = 'servico' AND os.status = 'CONCLUIDA'
           AND datetime(os.data_conclusao) >= ?1 AND datetime(os.data_conclusao) < ?2
         GROUP BY i.id, i.nome ORDER BY SUM(oi.quantidade) DESC, SUM(oi.subtotal) DESC, i.nome LIMIT 10",
    )?;
    let mais_realizados: Vec<ServicoRealizado> = stmt
        .query_map(params![ini, fim], |r| {
            Ok(ServicoRealizado { item_id: r.get(0)?, nome: r.get(1)?, quantidade: r.get(2)?, centavos: centavos(r.get::<_, f64>(3)?) })
        })?
        .collect::<Result<_, _>>()?;
    // Totais contam todos os serviços do período, não só os 10 primeiros.
    let (quantidade, soma): (i64, f64) = conn.query_row(
        "SELECT COALESCE(SUM(oi.quantidade), 0), COALESCE(SUM(oi.subtotal), 0) FROM ordem_itens oi
         JOIN itens i ON i.id = oi.item_id
         JOIN ordens_servico os ON os.id = oi.os_id
         WHERE i.tipo = 'servico' AND os.status = 'CONCLUIDA'
           AND datetime(os.data_conclusao) >= ?1 AND datetime(os.data_conclusao) < ?2",
        params![ini, fim],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok(Servicos { periodo: p.texto(), quantidade, centavos: centavos(soma), mais_realizados })
}

// ---------- Produtividade ----------

#[derive(Debug, Clone, Serialize)]
pub struct Produtividade {
    pub funcionario_id: i64,
    /// Funcionário RESPONSÁVEL pela OS (quem a abriu e responde por ela). Não é quem executou o serviço.
    pub responsavel_nome: String,
    pub ordens_concluidas: i64,
    /// Quantidade de serviços lançados nas ordens desse responsável.
    pub servicos_lancados: i64,
    pub centavos: i64,
}

pub fn produtividade(conn: &Connection, p: &Periodo) -> AppResult<Vec<Produtividade>> {
    let (ini, fim) = p.limites_instante();
    let mut stmt = conn.prepare(
        "SELECT f.id, f.nome, COUNT(os.id), COALESCE(SUM(os.valor_total), 0),
           (SELECT COALESCE(SUM(oi.quantidade), 0) FROM ordem_itens oi
              JOIN itens i ON i.id = oi.item_id
              JOIN ordens_servico x ON x.id = oi.os_id
             WHERE i.tipo = 'servico' AND x.funcionario_id = f.id AND x.status = 'CONCLUIDA'
               AND datetime(x.data_conclusao) >= ?1 AND datetime(x.data_conclusao) < ?2)
         FROM ordens_servico os
         JOIN funcionarios f ON f.id = os.funcionario_id
         WHERE os.status = 'CONCLUIDA' AND datetime(os.data_conclusao) >= ?1 AND datetime(os.data_conclusao) < ?2
         GROUP BY f.id, f.nome
         ORDER BY SUM(os.valor_total) DESC, f.nome",
    )?;
    let linhas = stmt
        .query_map(params![ini, fim], |r| {
            Ok(Produtividade {
                funcionario_id: r.get(0)?,
                responsavel_nome: r.get(1)?,
                ordens_concluidas: r.get(2)?,
                centavos: centavos(r.get::<_, f64>(3)?),
                servicos_lancados: r.get(4)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(linhas)
}

// ---------- Estoque ----------

#[derive(Debug, Clone, Serialize)]
pub struct ItemEstoque {
    pub id: i64,
    pub nome: String,
    pub quantidade_atual: i64,
    pub estoque_minimo: i64,
}

/// Movimento de peça no período: saída (ordens e vendas) ou entrada (compras).
#[derive(Debug, Clone, Serialize)]
pub struct MovimentoPeca {
    pub id: i64,
    pub nome: String,
    pub quantidade: i64,
    pub centavos: i64,
    pub quantidade_atual: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Estoque {
    pub periodo: String,
    /// Peças com saldo igual ou abaixo do mínimo, agora.
    pub saldo_baixo: Vec<ItemEstoque>,
    /// Saídas: peças baixadas em ordens e vendas concluídas (valor pelo preço de venda).
    pub saidas: Vec<MovimentoPeca>,
    pub saidas_quantidade: i64,
    pub saidas_centavos: i64,
    /// Entradas: peças recebidas em compras lançadas no período (valor pelo custo da compra).
    pub entradas: Vec<MovimentoPeca>,
    pub entradas_quantidade: i64,
    pub entradas_centavos: i64,
}

pub fn estoque(conn: &Connection, p: &Periodo) -> AppResult<Estoque> {
    let (ini, fim) = p.limites_instante();
    let (de, ate) = p.datas();
    let mut stmt = conn.prepare(
        "SELECT id, nome, quantidade, estoque_minimo FROM itens
         WHERE tipo = 'peca' AND quantidade <= estoque_minimo ORDER BY quantidade, nome",
    )?;
    let saldo_baixo = stmt
        .query_map([], |r| Ok(ItemEstoque { id: r.get(0)?, nome: r.get(1)?, quantidade_atual: r.get(2)?, estoque_minimo: r.get(3)? }))?
        .collect::<Result<_, _>>()?;

    let mut stmt = conn.prepare(
        "SELECT i.id, i.nome, SUM(oi.quantidade), SUM(oi.subtotal), i.quantidade FROM ordem_itens oi
         JOIN itens i ON i.id = oi.item_id
         JOIN ordens_servico os ON os.id = oi.os_id
         WHERE i.tipo = 'peca' AND os.status = 'CONCLUIDA'
           AND datetime(os.data_conclusao) >= ?1 AND datetime(os.data_conclusao) < ?2
         GROUP BY i.id, i.nome, i.quantidade ORDER BY SUM(oi.quantidade) DESC, i.nome",
    )?;
    let saidas: Vec<MovimentoPeca> = stmt
        .query_map(params![ini, fim], |r| {
            Ok(MovimentoPeca {
                id: r.get(0)?,
                nome: r.get(1)?,
                quantidade: r.get(2)?,
                centavos: centavos(r.get::<_, f64>(3)?),
                quantidade_atual: r.get(4)?,
            })
        })?
        .collect::<Result<_, _>>()?;

    let mut stmt = conn.prepare(
        "SELECT i.id, i.nome, SUM(ci.quantidade), SUM(ci.subtotal_centavos), i.quantidade FROM compra_itens ci
         JOIN compras c ON c.id = ci.compra_id
         JOIN itens i ON i.id = ci.item_id
         WHERE c.data >= ?1 AND c.data <= ?2
         GROUP BY i.id, i.nome, i.quantidade ORDER BY SUM(ci.quantidade) DESC, i.nome",
    )?;
    let entradas: Vec<MovimentoPeca> = stmt
        .query_map(params![de, ate], |r| {
            Ok(MovimentoPeca { id: r.get(0)?, nome: r.get(1)?, quantidade: r.get(2)?, centavos: r.get(3)?, quantidade_atual: r.get(4)? })
        })?
        .collect::<Result<_, _>>()?;

    Ok(Estoque {
        periodo: p.texto(),
        saldo_baixo,
        saidas_quantidade: saidas.iter().map(|s| s.quantidade).sum(),
        saidas_centavos: saidas.iter().map(|s| s.centavos).sum(),
        saidas,
        entradas_quantidade: entradas.iter().map(|s| s.quantidade).sum(),
        entradas_centavos: entradas.iter().map(|s| s.centavos).sum(),
        entradas,
    })
}

// ---------- Relatório completo ----------

#[derive(Debug, Clone, Serialize)]
pub struct Relatorio {
    pub periodo: String,
    pub de: String,
    pub ate: String,
    pub gerado_em: String,
    pub faturamento: Faturamento,
    pub recebimentos: Recebimentos,
    pub contas: Contas,
    pub lucro: Lucro,
    pub servicos: Servicos,
    pub produtividade: Vec<Produtividade>,
    pub estoque: Estoque,
}

/// Todos os relatórios do período de uma vez. `hoje` define "em aberto" e "vencido".
pub fn completo(conn: &Connection, p: &Periodo, hoje: NaiveDate) -> AppResult<Relatorio> {
    Ok(Relatorio {
        periodo: p.texto(),
        de: iso(p.de),
        ate: iso(p.ate),
        gerado_em: Local::now().format("%d/%m/%Y %H:%M").to_string(),
        faturamento: faturamento(conn, p)?,
        recebimentos: recebimentos(conn, p)?,
        contas: contas(conn, hoje)?,
        lucro: lucro(conn, p)?,
        servicos: servicos(conn, p)?,
        produtividade: produtividade(conn, p)?,
        estoque: estoque(conn, p)?,
    })
}
