//! Núcleo do sistema de oficina: banco SQLite, regras, segurança e sincronização.
//! Não depende de Tauri nem de interface, então pode ser testado isoladamente.

pub mod alteracoes;
pub mod caixa;
pub mod ativacao;
pub mod comandos;
pub mod fotos;
pub mod impressao;
pub mod registro;
pub mod relatorios;
pub mod rede_regras;
pub mod sync;
pub mod clientes;
pub mod compras;
pub mod fornecedores;
pub mod config;
pub mod dashboard;
pub mod db;
pub mod error;
pub mod financeiro;
pub mod fidelidade;
pub mod funcionarios;
pub mod hora;
pub mod itens;
pub mod ordens;
pub mod orcamentos;
pub mod painel;
pub mod seguranca;
pub mod sessao;
pub mod validation;
pub mod veiculos;

pub use error::{AppError, AppResult};
