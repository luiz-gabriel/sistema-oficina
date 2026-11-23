//! Segurança: senhas (argon2), chaves derivadas da ativação, HMAC e criptografia de transporte.
use argon2::password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

use crate::error::{AppError, AppResult, FieldError};

type HmacSha256 = Hmac<Sha256>;

/// Senhas muito comuns, recusadas mesmo com 8 caracteres.
const SENHAS_COMUNS: &[&str] = &[
    "12345678", "123456789", "11111111", "00000000", "password", "password1", "qwerty12", "qwerty123",
    "senha123", "senha1234", "abc12345", "oficina1", "oficina123", "mecanica1", "carro1234", "12341234",
];

/// Regra de senha: 8+ caracteres, com letra e número, não comum e sem o próprio login.
pub fn validar_senha(senha: &str, login: &str) -> AppResult<()> {
    let mut erros = Vec::new();
    let tamanho = senha.chars().count();
    if tamanho < 8 {
        erros.push("deve ter no mínimo 8 caracteres");
    }
    if !senha.chars().any(|c| c.is_alphabetic()) || !senha.chars().any(|c| c.is_numeric()) {
        erros.push("deve conter letras e números");
    }
    let minuscula = senha.to_lowercase();
    if SENHAS_COMUNS.contains(&minuscula.as_str()) {
        erros.push("é uma senha muito comum");
    }
    if !login.trim().is_empty() && minuscula.contains(&login.trim().to_lowercase()) {
        erros.push("não pode conter o login");
    }
    if erros.is_empty() {
        Ok(())
    } else {
        Err(AppError::Validation(vec![FieldError { field: "senha".into(), message: erros.join(", ") }]))
    }
}

pub fn hash_senha(senha: &str) -> AppResult<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(senha.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| AppError::Database(format!("falha ao gerar hash: {e}")))
}

pub fn verificar_senha(senha: &str, hash: &str) -> bool {
    PasswordHash::new(hash)
        .map(|h| Argon2::default().verify_password(senha.as_bytes(), &h).is_ok())
        .unwrap_or(false)
}

/// Normaliza um código de ativação (maiúsculas, sem espaços).
pub fn normalizar_codigo(codigo: &str) -> String {
    codigo.trim().to_uppercase().replace(' ', "")
}

/// SHA-256 em hexadecimal (usado para comparar linhas sem guardar o conteúdo).
pub fn sha256_hex(dados: &str) -> String {
    hex::encode(Sha256::digest(dados.as_bytes()))
}

/// Chave da rede derivada do código de ativação. O código em si nunca sai do computador.
pub fn chave_rede(codigo_ativacao: &str) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(None, normalizar_codigo(codigo_ativacao).as_bytes());
    let mut out = [0u8; 32];
    hk.expand(b"oficina-rede-v1", &mut out).expect("32 bytes é tamanho válido");
    out
}

pub fn hmac_hex(chave: &[u8], partes: &[&[u8]]) -> String {
    let mut m = <HmacSha256 as Mac>::new_from_slice(chave).expect("HMAC aceita qualquer tamanho de chave");
    for p in partes {
        m.update(p);
    }
    hex::encode(m.finalize().into_bytes())
}

/// Chave de sessão de uma conexão: depende da chave da rede e dos dois nonces (uma por conexão).
pub fn chave_sessao(chave: &[u8], nonce_cliente: &[u8], nonce_servidor: &[u8]) -> [u8; 32] {
    let mut salt = Vec::with_capacity(nonce_cliente.len() + nonce_servidor.len());
    salt.extend_from_slice(nonce_cliente);
    salt.extend_from_slice(nonce_servidor);
    let hk = Hkdf::<Sha256>::new(Some(&salt), chave);
    let mut out = [0u8; 32];
    hk.expand(b"oficina-sessao-v1", &mut out).expect("32 bytes é tamanho válido");
    out
}

/// Cifra uma mensagem com ChaCha20-Poly1305. `contador` garante que cada nonce é usado uma vez.
pub fn cifrar(chave_sessao: &[u8; 32], direcao: u8, contador: u64, dados: &[u8]) -> AppResult<Vec<u8>> {
    let cipher = ChaCha20Poly1305::new(Key::from_slice(chave_sessao));
    let nonce = nonce_de(direcao, contador);
    cipher.encrypt(&nonce, dados).map_err(|_| AppError::Database("falha ao cifrar".into()))
}

pub fn decifrar(chave_sessao: &[u8; 32], direcao: u8, contador: u64, dados: &[u8]) -> AppResult<Vec<u8>> {
    let cipher = ChaCha20Poly1305::new(Key::from_slice(chave_sessao));
    let nonce = nonce_de(direcao, contador);
    cipher.decrypt(&nonce, dados).map_err(|_| AppError::Conflict("mensagem não autenticada".into()))
}

fn nonce_de(direcao: u8, contador: u64) -> Nonce {
    let mut n = [0u8; 12];
    n[0] = direcao;
    n[4..].copy_from_slice(&contador.to_be_bytes());
    *Nonce::from_slice(&n)
}

pub fn nonce_aleatorio() -> [u8; 16] {
    use rand::RngCore;
    let mut b = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut b);
    b
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn senha_fraca_e_recusada() {
        assert!(validar_senha("abc", "ana").is_err());
        assert!(validar_senha("somenteletras", "ana").is_err());
        assert!(validar_senha("12345678", "ana").is_err());
        assert!(validar_senha("ana12345", "ana").is_err());
        assert!(validar_senha("oficina2026", "ana").is_ok());
    }

    #[test]
    fn hash_de_senha_verifica() {
        let h = hash_senha("oficina2026").unwrap();
        assert!(verificar_senha("oficina2026", &h));
        assert!(!verificar_senha("outra2026", &h));
    }

    #[test]
    fn cifra_ida_e_volta_e_detecta_adulteracao() {
        let chave = [7u8; 32];
        let c = cifrar(&chave, b'C', 1, b"ola").unwrap();
        assert_eq!(decifrar(&chave, b'C', 1, &c).unwrap(), b"ola");
        assert!(decifrar(&chave, b'C', 2, &c).is_err());
        assert!(decifrar(&chave, b'S', 1, &c).is_err());
    }

    #[test]
    fn chave_rede_depende_do_codigo() {
        assert_eq!(chave_rede("abcd-1234"), chave_rede(" ABCD-1234 "));
        assert_ne!(chave_rede("ABCD-1234"), chave_rede("ABCD-1235"));
    }
}
