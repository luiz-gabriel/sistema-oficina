// Token de sessão emitido pelo servidor: a identidade sai do token, não do que o cliente diz.
use std::time::{Duration, Instant};

use oficina_core::rede_regras::{Sessoes, VALIDADE_SESSAO};

#[test]
fn token_devolve_o_usuario_do_login() {
    let s = Sessoes::default();
    let agora = Instant::now();
    let token = s.emitir("ana", "administrador", "dev-1", agora);
    let sessao = s.validar(&token, "dev-1", agora).expect("token válido");
    assert_eq!(sessao.usuario, "ana");
    assert_eq!(sessao.perfil, "administrador");
}

#[test]
fn token_desconhecido_nao_vale() {
    let s = Sessoes::default();
    let agora = Instant::now();
    s.emitir("ana", "administrador", "dev-1", agora);
    assert!(s.validar("", "dev-1", agora).is_none());
    assert!(s.validar("0123abcd", "dev-1", agora).is_none());
}

#[test]
fn token_de_outro_dispositivo_nao_vale() {
    let s = Sessoes::default();
    let agora = Instant::now();
    let token = s.emitir("ana", "administrador", "dev-1", agora);
    assert!(s.validar(&token, "dev-2", agora).is_none());
}

#[test]
fn token_expira() {
    let s = Sessoes::default();
    let agora = Instant::now();
    let token = s.emitir("ana", "administrador", "dev-1", agora);
    assert!(s.validar(&token, "dev-1", agora + VALIDADE_SESSAO - Duration::from_secs(1)).is_some());
    assert!(s.validar(&token, "dev-1", agora + VALIDADE_SESSAO + Duration::from_secs(1)).is_none());
}

#[test]
fn encerrar_invalida_o_token() {
    let s = Sessoes::default();
    let agora = Instant::now();
    let token = s.emitir("ana", "administrador", "dev-1", agora);
    s.encerrar(&token);
    assert!(s.validar(&token, "dev-1", agora).is_none());
}

#[test]
fn tokens_sao_distintos_e_longos() {
    let s = Sessoes::default();
    let agora = Instant::now();
    let t1 = s.emitir("ana", "administrador", "dev-1", agora);
    let t2 = s.emitir("ana", "administrador", "dev-1", agora);
    assert_ne!(t1, t2);
    assert_eq!(t1.len(), 64);
}
