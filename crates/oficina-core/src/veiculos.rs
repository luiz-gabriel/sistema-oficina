use rusqlite::{params, Connection, Row};
use serde::{Deserialize, Serialize};

use crate::clientes;
use crate::error::{AppError, AppResult};
use crate::validation::{is_placa_valida, normalize_placa, Validator};

#[derive(Debug, Clone, Serialize)]
pub struct Veiculo {
    pub id: i64,
    pub cliente_id: i64,
    pub placa: String,
    pub marca: Option<String>,
    pub modelo: String,
    pub ano: Option<i64>,
    pub motor: Option<String>,
    pub cor: Option<String>,
    pub responsavel_id: Option<i64>,
    pub responsavel: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct VeiculoInput {
    pub cliente_id: Option<i64>,
    pub placa: Option<String>,
    pub marca: Option<String>,
    pub modelo: Option<String>,
    pub ano: Option<i64>,
    pub motor: Option<String>,
    pub cor: Option<String>,
    pub responsavel_id: Option<i64>,
}

const SELECT: &str = "SELECT v.id, v.cliente_id, v.placa, v.marca, v.modelo, v.ano, v.motor, v.cor, v.responsavel_id, f.nome \
    FROM veiculos v LEFT JOIN funcionarios f ON f.id = v.responsavel_id";

fn de_linha(r: &Row) -> rusqlite::Result<Veiculo> {
    Ok(Veiculo {
        id: r.get(0)?,
        cliente_id: r.get(1)?,
        placa: r.get(2)?,
        marca: r.get(3)?,
        modelo: r.get(4)?,
        ano: r.get(5)?,
        motor: r.get(6)?,
        cor: r.get(7)?,
        responsavel_id: r.get(8)?,
        responsavel: r.get(9)?,
    })
}

/// Dados já validados e normalizados, prontos para gravar.
struct Limpo {
    cliente_id: i64,
    placa: String,
    marca: Option<String>,
    modelo: String,
    ano: Option<i64>,
    motor: Option<String>,
    cor: Option<String>,
    responsavel_id: Option<i64>,
}

/// Valida e normaliza. Também confere se o cliente existe (erro claro, não de chave estrangeira).
fn validar(conn: &Connection, input: &VeiculoInput) -> AppResult<Limpo> {
    let mut v = Validator::new();
    let cliente_id = v.int("cliente_id", input.cliente_id, true, 1, i64::MAX);
    let modelo = v.text("modelo", input.modelo.as_deref(), true, 60);
    let marca = v.text("marca", input.marca.as_deref(), false, 60);
    let ano = v.int("ano", input.ano, false, 1900, 2100);
    let motor = v.text("motor", input.motor.as_deref(), false, 40);
    let cor = v.text("cor", input.cor.as_deref(), false, 30);
    let responsavel_id = input.responsavel_id;

    let placa = match input.placa.as_deref() {
        None => {
            v.fail("placa", "campo obrigatório");
            None
        }
        Some(raw) => {
            let p = normalize_placa(raw);
            if is_placa_valida(&p) {
                Some(p)
            } else {
                v.fail("placa", "placa inválida");
                None
            }
        }
    };
    v.finish()?;

    let cliente_id = cliente_id.expect("validado pelo Validator");
    clientes::buscar(conn, cliente_id)?;
    if let Some(rid) = responsavel_id {
        crate::funcionarios::buscar(conn, rid).map_err(|_| AppError::Validation(vec![crate::error::FieldError {
            field: "responsavel_id".into(),
            message: "Responsável não encontrado.".into(),
        }]))?;
    }
    Ok(Limpo {
        cliente_id,
        placa: placa.expect("validado pelo Validator"),
        marca,
        modelo: modelo.expect("validado pelo Validator"),
        ano,
        motor,
        cor,
        responsavel_id,
    })
}

pub fn listar(conn: &Connection) -> AppResult<Vec<Veiculo>> {
    let mut stmt = conn.prepare(&format!("{SELECT} ORDER BY v.placa"))?;
    let rows = stmt.query_map([], de_linha)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn buscar(conn: &Connection, id: i64) -> AppResult<Veiculo> {
    conn.query_row(&format!("{SELECT} WHERE v.id = ?1"), [id], de_linha)
        .map_err(|_| AppError::NotFound("Veículo não encontrado(a).".into()))
}

pub fn criar(conn: &Connection, input: &VeiculoInput) -> AppResult<Veiculo> {
    let d = validar(conn, input)?;
    conn.execute(
        "INSERT INTO veiculos (cliente_id, placa, marca, modelo, ano, motor, cor, responsavel_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![d.cliente_id, d.placa, d.marca, d.modelo, d.ano, d.motor, d.cor, d.responsavel_id],
    )
    .map_err(|e| match AppError::from(e) {
        AppError::Conflict(_) => AppError::Conflict("Já existe um veículo com essa placa.".into()),
        outro => outro,
    })?;
    buscar(conn, conn.last_insert_rowid())
}

pub fn atualizar(conn: &Connection, id: i64, input: &VeiculoInput) -> AppResult<Veiculo> {
    buscar(conn, id)?;
    let d = validar(conn, input)?;
    conn.execute(
        "UPDATE veiculos SET cliente_id=?1, placa=?2, marca=?3, modelo=?4, ano=?5, motor=?6, cor=?7, responsavel_id=?8 WHERE id=?9",
        params![d.cliente_id, d.placa, d.marca, d.modelo, d.ano, d.motor, d.cor, d.responsavel_id, id],
    )
    .map_err(|e| match AppError::from(e) {
        AppError::Conflict(_) => AppError::Conflict("Já existe um veículo com essa placa.".into()),
        outro => outro,
    })?;
    buscar(conn, id)
}

pub fn remover(conn: &Connection, id: i64) -> AppResult<()> {
    buscar(conn, id)?;
    conn.execute("DELETE FROM veiculos WHERE id = ?1", [id])?;
    Ok(())
}

// ---------- Histórico do veículo: quilometragem e dados de cada atendimento (tabela `ordem_km`) ----------

/// Maior quilometragem aceita (7 dígitos).
pub const KM_MAXIMO: i64 = 9_999_999;
/// Garantia em dias: até 10 anos.
const GARANTIA_MAXIMA_DIAS: i64 = 3650;

/// Dados do atendimento de uma OS. Campo vazio (None) não apaga o que já está gravado.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct DadosAtendimento {
    /// Quilometragem que o carro tinha quando entrou na oficina.
    pub km_entrada: Option<i64>,
    /// Quilometragem que o carro tinha quando foi entregue.
    pub km_entrega: Option<i64>,
    /// Data prevista para ficar pronto (AAAA-MM-DD).
    pub prazo_previsto: Option<String>,
    /// Garantia do serviço, em dias, contada a partir da entrega.
    pub garantia_dias: Option<i64>,
    /// Este atendimento foi uma revisão programada.
    pub revisao: Option<bool>,
    pub observacoes_tecnicas: Option<String>,
}

/// Uma ordem ou orçamento do veículo, com os dados do atendimento.
#[derive(Debug, Clone, Serialize)]
pub struct EntradaHistorico {
    pub ordem: crate::ordens::Ordem,
    pub km_entrada: Option<i64>,
    pub km_entrega: Option<i64>,
    pub prazo_previsto: Option<String>,
    pub garantia_dias: Option<i64>,
    /// Data até a qual vale a garantia (entrega + dias), no formato AAAA-MM-DD.
    pub garantia_ate: Option<String>,
    pub revisao: bool,
    pub observacoes_tecnicas: Option<String>,
}

/// Histórico completo do carro: o mais recente primeiro.
#[derive(Debug, Clone, Serialize)]
pub struct HistoricoVeiculo {
    pub veiculo: Veiculo,
    /// Maior quilometragem já registrada neste carro.
    pub km_mais_recente: Option<i64>,
    pub entradas: Vec<EntradaHistorico>,
}

#[derive(Debug, Clone, Default)]
struct DadosGravados {
    km_entrada: Option<i64>,
    km_entrega: Option<i64>,
    prazo_previsto: Option<String>,
    garantia_dias: Option<i64>,
    revisao: bool,
    observacoes_tecnicas: Option<String>,
}

fn dados_gravados(conn: &Connection, os_id: i64) -> AppResult<DadosGravados> {
    let linha = conn.query_row(
        "SELECT km_entrada, km_entrega, prazo_previsto, garantia_dias, revisao, observacoes_tecnicas FROM ordem_km WHERE os_id = ?1",
        [os_id],
        |r| Ok(DadosGravados {
            km_entrada: r.get(0)?,
            km_entrega: r.get(1)?,
            prazo_previsto: r.get(2)?,
            garantia_dias: r.get(3)?,
            revisao: r.get::<_, i64>(4)? == 1,
            observacoes_tecnicas: r.get(5)?,
        }),
    );
    match linha {
        Ok(d) => Ok(d),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(DadosGravados::default()),
        Err(e) => Err(e.into()),
    }
}

/// Grava os dados do atendimento de UMA ordem. Não altera as quilometragens de outras ordens.
pub fn salvar_dados_ordem(conn: &Connection, os_id: i64, input: &DadosAtendimento) -> AppResult<()> {
    let ordem = crate::ordens::buscar(conn, os_id)?;
    if ordem.tipo != "OS" {
        return Err(AppError::Conflict("Venda de balcão não tem veículo, então não há quilometragem para registrar.".into()));
    }
    let mut v = Validator::new();
    v.int("km_entrada", input.km_entrada, false, 0, KM_MAXIMO);
    v.int("km_entrega", input.km_entrega, false, 0, KM_MAXIMO);
    v.int("garantia_dias", input.garantia_dias, false, 0, GARANTIA_MAXIMA_DIAS);
    v.text("observacoes_tecnicas", input.observacoes_tecnicas.as_deref(), false, 1000);
    let prazo = match input.prazo_previsto.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        None => None,
        Some(t) => {
            if chrono::NaiveDate::parse_from_str(t, "%Y-%m-%d").is_err() {
                v.fail("prazo_previsto", "data inválida");
            }
            Some(t.to_string())
        }
    };
    v.finish()?;

    let atual = dados_gravados(conn, os_id)?;
    let km_entrada = input.km_entrada.or(atual.km_entrada);
    let km_entrega = input.km_entrega.or(atual.km_entrega);
    if let (Some(e), Some(s)) = (km_entrada, km_entrega) {
        if s < e {
            return Err(AppError::Validation(vec![crate::error::FieldError {
                field: "km_entrega".into(),
                message: "A quilometragem da entrega não pode ser menor que a da entrada.".into(),
            }]));
        }
    }
    let novo = DadosGravados {
        km_entrada,
        km_entrega,
        prazo_previsto: prazo.or(atual.prazo_previsto),
        garantia_dias: input.garantia_dias.or(atual.garantia_dias),
        revisao: input.revisao.unwrap_or(atual.revisao),
        observacoes_tecnicas: input
            .observacoes_tecnicas
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .or(atual.observacoes_tecnicas),
    };
    conn.execute(
        "INSERT INTO ordem_km (os_id, km_entrada, km_entrega, prazo_previsto, garantia_dias, revisao, observacoes_tecnicas)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(os_id) DO UPDATE SET
           km_entrada = excluded.km_entrada, km_entrega = excluded.km_entrega,
           prazo_previsto = excluded.prazo_previsto, garantia_dias = excluded.garantia_dias,
           revisao = excluded.revisao, observacoes_tecnicas = excluded.observacoes_tecnicas",
        params![
            os_id, novo.km_entrada, novo.km_entrega, novo.prazo_previsto, novo.garantia_dias,
            if novo.revisao { 1 } else { 0 }, novo.observacoes_tecnicas
        ],
    )?;
    Ok(())
}

/// Histórico do veículo: ordens e orçamentos (mais recentes primeiro), com peças, serviços, datas, garantia e revisões.
pub fn historico(conn: &Connection, veiculo_id: i64) -> AppResult<HistoricoVeiculo> {
    let veiculo = buscar(conn, veiculo_id)?;
    let ids: Vec<i64> = {
        let mut stmt = conn.prepare("SELECT id FROM ordens_servico WHERE veiculo_id = ?1 ORDER BY data_entrada DESC, id DESC")?;
        let ids = stmt.query_map([veiculo_id], |r| r.get(0))?.collect::<Result<_, _>>()?;
        ids
    };
    let mut entradas = Vec::with_capacity(ids.len());
    for id in ids {
        let ordem = crate::ordens::buscar(conn, id)?;
        let d = dados_gravados(conn, id)?;
        let garantia_ate = match (d.garantia_dias, ordem.data_conclusao.as_deref()) {
            (Some(dias), Some(entrega)) => chrono::DateTime::parse_from_rfc3339(entrega)
                .ok()
                .and_then(|t| t.date_naive().checked_add_days(chrono::Days::new(dias.max(0) as u64)))
                .map(|d| d.format("%Y-%m-%d").to_string()),
            _ => None,
        };
        entradas.push(EntradaHistorico {
            ordem,
            km_entrada: d.km_entrada,
            km_entrega: d.km_entrega,
            prazo_previsto: d.prazo_previsto,
            garantia_dias: d.garantia_dias,
            garantia_ate,
            revisao: d.revisao,
            observacoes_tecnicas: d.observacoes_tecnicas,
        });
    }
    let km_mais_recente = entradas.iter().flat_map(|e| [e.km_entrada, e.km_entrega]).flatten().max();
    Ok(HistoricoVeiculo { veiculo, km_mais_recente, entradas })
}
