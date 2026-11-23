//! Validação acumulativa: coleta todos os erros de um formulário de uma vez.

use crate::error::{AppError, AppResult, FieldError};

#[derive(Default)]
pub struct Validator {
    errors: Vec<FieldError>,
}

impl Validator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registra um erro manual (para regras específicas, como formato de placa).
    pub fn fail(&mut self, field: &str, message: impl Into<String>) {
        self.errors.push(FieldError { field: field.into(), message: message.into() });
    }

    /// Texto. Vazio vira None; obrigatório vazio gera erro.
    pub fn text(&mut self, field: &str, value: Option<&str>, required: bool, max_len: usize) -> Option<String> {
        let trimmed = value.map(str::trim).filter(|s| !s.is_empty());
        match trimmed {
            None if required => {
                self.fail(field, "campo obrigatório");
                None
            }
            None => None,
            Some(s) if s.chars().count() > max_len => {
                self.fail(field, format!("máximo de {max_len} caracteres"));
                None
            }
            Some(s) => Some(s.to_string()),
        }
    }

    /// Inteiro com faixa opcional.
    pub fn int(&mut self, field: &str, value: Option<i64>, required: bool, min: i64, max: i64) -> Option<i64> {
        match value {
            None if required => {
                self.fail(field, "campo obrigatório");
                None
            }
            None => None,
            Some(n) if n < min || n > max => {
                self.fail(field, format!("deve estar entre {min} e {max}"));
                None
            }
            Some(n) => Some(n),
        }
    }

    /// Valor monetário não negativo, arredondado para centavos.
    pub fn money(&mut self, field: &str, value: Option<f64>, required: bool) -> Option<f64> {
        match value {
            None if required => {
                self.fail(field, "campo obrigatório");
                None
            }
            None => None,
            Some(v) if !v.is_finite() || v < 0.0 => {
                self.fail(field, "deve ser um valor maior ou igual a zero");
                None
            }
            Some(v) => Some(round2(v)),
        }
    }

    pub fn finish(self) -> AppResult<()> {
        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(AppError::Validation(self.errors))
        }
    }
}

/// Placa: remove hífen/espaços e põe em maiúscula. Aceita formato antigo (ABC1234) e Mercosul (ABC1D23).
pub fn normalize_placa(raw: &str) -> String {
    raw.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_uppercase()).collect()
}

pub fn is_placa_valida(placa: &str) -> bool {
    let b = placa.as_bytes();
    b.len() == 7
        && b[0..3].iter().all(u8::is_ascii_uppercase)
        && b[3].is_ascii_digit()
        && b[4].is_ascii_alphanumeric()
        && b[5].is_ascii_digit()
        && b[6].is_ascii_digit()
}

pub fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}
