//! Ordem de serviço: regras puras em `rules`, orquestração com o banco em `service`.
pub mod rules;
pub mod service;

pub use rules::Status;
pub use service::{
    alterar_status, buscar, concluir_com_pagamento, concluir_servico, criar, criar_balcao, excluir, listar, substituir_itens, BalcaoInput, Ordem,
    OrdemInput, LinhaInput, LinhaOrdem,
};
