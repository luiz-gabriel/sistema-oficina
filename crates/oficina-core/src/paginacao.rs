//! Paginação das listas grandes (clientes, veículos, peças, ordens).
//!
//! A tela pede um bloco de `limite` registros a partir de `deslocamento`. Quando a pessoa chega ao fim
//! do bloco carregado, a tela pede o próximo. Os limites ficam aqui para que nenhum pedido traga a tabela inteira.

/// Tamanho do bloco quando a tela não informa o seu.
pub const TAMANHO_PADRAO: i64 = 80;
/// Maior bloco aceito numa única resposta.
pub const TAMANHO_MAXIMO: i64 = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pagina {
    pub limite: i64,
    pub deslocamento: i64,
}

impl Pagina {
    /// Aceita o que veio da tela e corrige valores fora da faixa (ausente, zero, negativo ou grande demais).
    pub fn nova(limite: Option<i64>, deslocamento: Option<i64>) -> Pagina {
        Pagina {
            limite: limite.unwrap_or(TAMANHO_PADRAO).clamp(1, TAMANHO_MAXIMO),
            deslocamento: deslocamento.unwrap_or(0).max(0),
        }
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn valores_ausentes_ganham_o_padrao() {
        assert_eq!(Pagina::nova(None, None), Pagina { limite: TAMANHO_PADRAO, deslocamento: 0 });
    }

    #[test]
    fn valores_fora_da_faixa_sao_corrigidos() {
        assert_eq!(Pagina::nova(Some(0), Some(-5)), Pagina { limite: 1, deslocamento: 0 });
        assert_eq!(Pagina::nova(Some(999_999), Some(40)), Pagina { limite: TAMANHO_MAXIMO, deslocamento: 40 });
    }
}
