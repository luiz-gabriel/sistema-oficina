//! Regras puras da OS (sem banco), fáceis de testar.
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Status {
    Criada,
    EmAndamento,
    /// Serviço concluído, aguardando o pagamento (pode receber sinal, parcial ou total).
    AguardandoPagamento,
    Concluida,
    Cancelada,
}

impl Status {
    pub fn como_texto(self) -> &'static str {
        match self {
            Status::Criada => "CRIADA",
            Status::EmAndamento => "EM_ANDAMENTO",
            Status::AguardandoPagamento => "AGUARDANDO_PAGAMENTO",
            Status::Concluida => "CONCLUIDA",
            Status::Cancelada => "CANCELADA",
        }
    }

    pub fn de_texto(s: &str) -> Option<Status> {
        match s {
            "CRIADA" => Some(Status::Criada),
            "EM_ANDAMENTO" => Some(Status::EmAndamento),
            "AGUARDANDO_PAGAMENTO" => Some(Status::AguardandoPagamento),
            "CONCLUIDA" => Some(Status::Concluida),
            "CANCELADA" => Some(Status::Cancelada),
            _ => None,
        }
    }

    /// Máquina de estados: CONCLUIDA e CANCELADA são finais.
    /// O pagamento não muda o status: a OS só vira CONCLUIDA quando estiver totalmente paga.
    pub fn pode_transitar_para(self, para: Status) -> bool {
        matches!(
            (self, para),
            (Status::Criada, Status::EmAndamento)
                | (Status::Criada, Status::Cancelada)
                | (Status::EmAndamento, Status::AguardandoPagamento)
                | (Status::EmAndamento, Status::Cancelada)
                | (Status::AguardandoPagamento, Status::Concluida)
        )
    }

    pub fn permite_editar_itens(self) -> bool {
        self == Status::Criada
    }

    pub fn permite_excluir(self) -> bool {
        matches!(self, Status::Criada | Status::Cancelada)
    }
}

pub fn validar_transicao(de: Status, para: Status) -> AppResult<()> {
    if de.pode_transitar_para(para) {
        Ok(())
    } else {
        Err(AppError::Conflict(format!(
            "Transição inválida: {} → {}.",
            de.como_texto(),
            para.como_texto()
        )))
    }
}

pub fn subtotal(quantidade: i64, preco_unitario: f64) -> f64 {
    crate::validation::round2(quantidade as f64 * preco_unitario)
}

pub fn total(subtotais: &[f64]) -> f64 {
    crate::validation::round2(subtotais.iter().sum())
}
