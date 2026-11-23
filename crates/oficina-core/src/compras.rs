//! Compras de fornecedores. Registrar uma compra dá entrada no estoque e atualiza o custo médio
//! do item na MESMA transação: se qualquer etapa falhar, nada é gravado.
use chrono::{Local, NaiveDate, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::validation::Validator;

/// Formas de pagamento aceitas. "a pagar" gera conta a pagar em vez de já marcar como paga.
pub const FORMAS: &[&str] = &["dinheiro", "pix", "cartao", "boleto", "transferencia", "a pagar"];
pub const A_PAGAR: &str = "a pagar";

#[derive(Debug, Clone, Deserialize, Default)]
pub struct LinhaInput {
    pub item_id: Option<i64>,
    pub quantidade: Option<i64>,
    pub custo_unitario_centavos: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct CompraInput {
    pub fornecedor_id: Option<i64>,
    /// AAAA-MM-DD. Vazio usa a data de hoje.
    pub data: Option<String>,
    pub pagamento: Option<String>,
    /// AAAA-MM-DD. Obrigatório quando a compra é "a pagar".
    pub vencimento: Option<String>,
    #[serde(default)]
    pub itens: Vec<LinhaInput>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LinhaCompra {
    pub item_id: i64,
    pub item_nome: String,
    pub quantidade: i64,
    pub custo_unitario_centavos: i64,
    pub subtotal_centavos: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Compra {
    pub id: i64,
    pub fornecedor_id: i64,
    pub fornecedor_nome: String,
    pub data: String,
    pub total_centavos: i64,
    pub pagamento: String,
    pub paga: bool,
    pub vencimento: Option<String>,
    pub itens: Vec<LinhaCompra>,
}

/// Uma compra de um item, vista pelo lado do item (histórico de custo).
#[derive(Debug, Clone, Serialize)]
pub struct CompraDoItem {
    pub compra_id: i64,
    pub data: String,
    pub fornecedor_id: i64,
    pub fornecedor_nome: String,
    pub quantidade: i64,
    pub custo_unitario_centavos: i64,
    pub subtotal_centavos: i64,
}

/// Custo médio ponderado pelas entradas, em centavos inteiros, arredondado para baixo.
///
/// novo = (estoque × custo_atual + qtd × custo_compra) / (estoque + qtd)
///
/// Estoque zero (ou negativo) faz o custo atual valer zero, então o novo custo é o da compra.
/// As contas usam i128 para não estourar com valores grandes.
pub fn custo_medio_novo(estoque_atual: i64, custo_atual_centavos: i64, qtd: i64, custo_compra_centavos: i64) -> i64 {
    let estoque = i128::from(estoque_atual.max(0));
    let custo_atual = if estoque == 0 { 0 } else { i128::from(custo_atual_centavos.max(0)) };
    let den = estoque + i128::from(qtd);
    if qtd <= 0 || den <= 0 {
        return custo_atual_centavos;
    }
    let num = estoque * custo_atual + i128::from(qtd) * i128::from(custo_compra_centavos);
    (num / den) as i64
}

/// Preço de custo guardado em reais (REAL) para centavos inteiros.
fn centavos_de_reais(reais: f64) -> i64 {
    (reais * 100.0).round() as i64
}

struct Linha {
    item_id: i64,
    quantidade: i64,
    custo: i64,
    subtotal: i64,
}

struct Validada {
    fornecedor_id: i64,
    data: String,
    pagamento: String,
    vencimento: Option<String>,
    linhas: Vec<Linha>,
    total: i64,
}

fn data_valida(s: &str) -> bool {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
}

fn validar(conn: &Connection, input: &CompraInput) -> AppResult<Validada> {
    let mut v = Validator::new();

    let fornecedor_id = v.int("fornecedor_id", input.fornecedor_id, true, 1, i64::MAX);
    if let Some(f) = fornecedor_id {
        let existe: Option<i64> = conn
            .query_row("SELECT id FROM fornecedores WHERE id = ?1", [f], |r| r.get(0))
            .optional()?;
        if existe.is_none() {
            v.fail("fornecedor_id", "fornecedor não encontrado");
        }
    }

    let data = match input.data.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        None => Local::now().date_naive().format("%Y-%m-%d").to_string(),
        Some(s) if data_valida(s) => s.to_string(),
        Some(_) => {
            v.fail("data", "data inválida (use AAAA-MM-DD)");
            String::new()
        }
    };

    let pagamento = match input.pagamento.as_deref().map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()) {
        None => {
            v.fail("pagamento", "campo obrigatório");
            String::new()
        }
        Some(p) if FORMAS.contains(&p.as_str()) => p,
        Some(_) => {
            v.fail("pagamento", "escolha uma forma de pagamento ou 'a pagar'");
            String::new()
        }
    };

    let vencimento = if pagamento == A_PAGAR {
        match input.vencimento.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            None => {
                v.fail("vencimento", "informe o vencimento da conta a pagar");
                None
            }
            Some(s) if data_valida(s) => Some(s.to_string()),
            Some(_) => {
                v.fail("vencimento", "data inválida (use AAAA-MM-DD)");
                None
            }
        }
    } else {
        None
    };

    if input.itens.is_empty() {
        v.fail("itens", "informe ao menos uma peça");
    }
    let mut linhas = Vec::new();
    let mut total: i64 = 0;
    for (n, l) in input.itens.iter().enumerate() {
        let rotulo = format!("Linha {}", n + 1);
        let Some(item_id) = l.item_id else {
            v.fail("itens", format!("{rotulo}: escolha a peça"));
            continue;
        };
        let tipo: Option<String> = conn
            .query_row("SELECT tipo FROM itens WHERE id = ?1", [item_id], |r| r.get(0))
            .optional()?;
        match tipo.as_deref() {
            None => {
                v.fail("itens", format!("{rotulo}: peça não encontrada"));
                continue;
            }
            Some("peca") => {}
            Some(_) => {
                v.fail("itens", format!("{rotulo}: serviço não entra em compra (só peças têm estoque)"));
                continue;
            }
        }
        let quantidade = match l.quantidade {
            Some(q) if q > 0 => q,
            _ => {
                v.fail("itens", format!("{rotulo}: quantidade deve ser maior que zero"));
                continue;
            }
        };
        let custo = match l.custo_unitario_centavos {
            Some(c) if c >= 0 => c,
            _ => {
                v.fail("itens", format!("{rotulo}: custo unitário inválido"));
                continue;
            }
        };
        let Some(subtotal) = quantidade.checked_mul(custo) else {
            v.fail("itens", format!("{rotulo}: valor muito alto"));
            continue;
        };
        let Some(novo_total) = total.checked_add(subtotal) else {
            v.fail("itens", format!("{rotulo}: total da compra muito alto"));
            continue;
        };
        total = novo_total;
        linhas.push(Linha { item_id, quantidade, custo, subtotal });
    }

    v.finish()?;
    Ok(Validada {
        fornecedor_id: fornecedor_id.expect("validado"),
        data,
        pagamento,
        vencimento,
        linhas,
        total,
    })
}

/// Grava a compra, as linhas, o estoque, o custo médio e, se for a prazo, a conta em `contas_pagar`.
/// Roda dentro de uma transação: quem chama deve usar `registrar`, que cuida disso.
/// Compra a prazo de valor zero não tem dívida: fica marcada como paga e não gera conta (valor da conta precisa ser > 0).
fn gravar(conn: &Connection, v: &Validada, usuario: &str) -> AppResult<i64> {
    let a_prazo = v.pagamento == A_PAGAR;
    let paga = !a_prazo || v.total == 0;
    conn.execute(
        "INSERT INTO compras (fornecedor_id, data, total_centavos, pagamento, paga, criado_em)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![v.fornecedor_id, v.data, v.total, v.pagamento, i64::from(paga), Utc::now().to_rfc3339()],
    )?;
    let compra_id = conn.last_insert_rowid();

    for l in &v.linhas {
        conn.execute(
            "INSERT INTO compra_itens (compra_id, item_id, quantidade, custo_unitario_centavos, subtotal_centavos)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![compra_id, l.item_id, l.quantidade, l.custo, l.subtotal],
        )?;
        let (estoque, preco_custo): (i64, f64) = conn.query_row(
            "SELECT quantidade, preco_custo FROM itens WHERE id = ?1",
            [l.item_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let novo_custo = custo_medio_novo(estoque, centavos_de_reais(preco_custo), l.quantidade, l.custo);
        conn.execute(
            "UPDATE itens SET quantidade = quantidade + ?1, preco_custo = ?2 WHERE id = ?3",
            params![l.quantidade, novo_custo as f64 / 100.0, l.item_id],
        )?;
    }

    if let (Some(venc), true) = (&v.vencimento, v.total > 0) {
        let fornecedor: String =
            conn.query_row("SELECT nome FROM fornecedores WHERE id = ?1", [v.fornecedor_id], |r| r.get(0))?;
        conn.execute(
            "INSERT INTO contas_pagar (descricao, fornecedor, compra_id, valor_centavos, vencimento, data, usuario)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![format!("Compra #{compra_id}"), fornecedor, compra_id, v.total, venc, v.data, usuario],
        )?;
    }
    Ok(compra_id)
}

/// Registra a compra de forma atômica: entrada de estoque, custo médio e conta a pagar.
/// Se qualquer etapa falhar, a transação é desfeita e nada fica gravado pela metade.
/// `usuario` fica registrado na conta a pagar gerada.
pub fn registrar(conn: &Connection, input: &CompraInput, usuario: &str) -> AppResult<Compra> {
    let validada = validar(conn, input)?;
    let tx = conn.unchecked_transaction()?;
    let id = gravar(&tx, &validada, usuario)?;
    let compra = buscar(&tx, id)?;
    tx.commit()?;
    Ok(compra)
}

/// Situação da compra: se tem conta a pagar, quitada quando a soma dos pagamentos chega ao valor da conta;
/// se não tem conta (à vista), o próprio campo `paga`.
pub fn buscar(conn: &Connection, id: i64) -> AppResult<Compra> {
    let mut cab = conn
        .query_row(
            "SELECT c.id, c.fornecedor_id, f.nome, c.data, c.total_centavos, c.pagamento, c.paga,
                    cp.vencimento, cp.valor_centavos, COALESCE(pg.pago, 0)
             FROM compras c
             JOIN fornecedores f ON f.id = c.fornecedor_id
             LEFT JOIN contas_pagar cp ON cp.compra_id = c.id
             LEFT JOIN (SELECT conta_id, SUM(valor_centavos) AS pago FROM pagamentos_pagar GROUP BY conta_id) pg
               ON pg.conta_id = cp.id
             WHERE c.id = ?1",
            [id],
            |r| {
                let paga_coluna = r.get::<_, i64>(6)? != 0;
                let valor_conta: Option<i64> = r.get(8)?;
                let pago: i64 = r.get(9)?;
                Ok(Compra {
                    id: r.get(0)?,
                    fornecedor_id: r.get(1)?,
                    fornecedor_nome: r.get(2)?,
                    data: r.get(3)?,
                    total_centavos: r.get(4)?,
                    pagamento: r.get(5)?,
                    paga: match valor_conta {
                        Some(valor) => pago >= valor,
                        None => paga_coluna,
                    },
                    vencimento: r.get(7)?,
                    itens: Vec::new(),
                })
            },
        )
        .optional()?
        .ok_or_else(|| AppError::NotFound("Compra não encontrada.".into()))?;
    let mut stmt = conn.prepare(
        "SELECT ci.item_id, i.nome, ci.quantidade, ci.custo_unitario_centavos, ci.subtotal_centavos
         FROM compra_itens ci
         JOIN itens i ON i.id = ci.item_id
         WHERE ci.compra_id = ?1
         ORDER BY ci.id",
    )?;
    cab.itens = stmt
        .query_map([id], |r| {
            Ok(LinhaCompra {
                item_id: r.get(0)?,
                item_nome: r.get(1)?,
                quantidade: r.get(2)?,
                custo_unitario_centavos: r.get(3)?,
                subtotal_centavos: r.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(cab)
}

fn ids_de(conn: &Connection, sql: &str, p: i64) -> AppResult<Vec<i64>> {
    let mut stmt = conn.prepare(sql)?;
    let ids = stmt.query_map([p], |r| r.get(0))?.collect::<Result<Vec<i64>, _>>()?;
    Ok(ids)
}

/// Todas as compras, da mais recente para a mais antiga.
pub fn listar(conn: &Connection) -> AppResult<Vec<Compra>> {
    let mut stmt = conn.prepare("SELECT id FROM compras ORDER BY data DESC, id DESC")?;
    let ids = stmt.query_map([], |r| r.get::<_, i64>(0))?.collect::<Result<Vec<_>, _>>()?;
    ids.into_iter().map(|id| buscar(conn, id)).collect()
}

/// Compras de um fornecedor, com as linhas de cada uma (mais recente primeiro).
pub fn historico_fornecedor(conn: &Connection, fornecedor_id: i64) -> AppResult<Vec<Compra>> {
    crate::fornecedores::buscar(conn, fornecedor_id)?;
    let ids = ids_de(conn, "SELECT id FROM compras WHERE fornecedor_id = ?1 ORDER BY data DESC, id DESC", fornecedor_id)?;
    ids.into_iter().map(|id| buscar(conn, id)).collect()
}

/// Compras em que um item entrou (custo de cada entrada), mais recente primeiro.
pub fn historico_item(conn: &Connection, item_id: i64) -> AppResult<Vec<CompraDoItem>> {
    let mut stmt = conn.prepare(
        "SELECT ci.compra_id, c.data, c.fornecedor_id, f.nome, ci.quantidade, ci.custo_unitario_centavos, ci.subtotal_centavos
         FROM compra_itens ci
         JOIN compras c ON c.id = ci.compra_id
         JOIN fornecedores f ON f.id = c.fornecedor_id
         WHERE ci.item_id = ?1
         ORDER BY c.data DESC, ci.compra_id DESC, ci.id DESC",
    )?;
    let linhas = stmt
        .query_map([item_id], |r| {
            Ok(CompraDoItem {
                compra_id: r.get(0)?,
                data: r.get(1)?,
                fornecedor_id: r.get(2)?,
                fornecedor_nome: r.get(3)?,
                quantidade: r.get(4)?,
                custo_unitario_centavos: r.get(5)?,
                subtotal_centavos: r.get(6)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(linhas)
}
