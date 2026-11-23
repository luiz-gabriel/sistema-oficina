//! Entrada no sistema: senha certa, bloqueio após 3 erros e desbloqueio depois do prazo.
use chrono::{Duration, Utc};
use oficina_core::db;
use oficina_core::funcionarios::{self, AcessoInput, Entrada, FuncionarioInput};

fn cenario() -> rusqlite::Connection {
    let conn = db::open_in_memory().unwrap();
    let f = funcionarios::criar(&conn, &FuncionarioInput { nome: Some("Ana".into()), ..Default::default() }).unwrap();
    funcionarios::dar_acesso(&conn, f.id, &AcessoInput { login: Some("ana".into()), senha: Some("senha12345".into()), senha_hash: None, perfil: Some("funcionario".into()) }).unwrap();
    conn
}

#[test]
fn senha_certa_entra_e_zera_as_falhas() {
    let conn = cenario();
    let agora = Utc::now();
    assert!(matches!(funcionarios::entrar(&conn, "ana", "errada", agora).unwrap(), Entrada::Falhou));
    assert!(matches!(funcionarios::entrar(&conn, "ana", "senha12345", agora).unwrap(), Entrada::Ok(_)));
    // Depois de entrar, as falhas anteriores não contam mais.
    assert!(matches!(funcionarios::entrar(&conn, "ana", "errada", agora).unwrap(), Entrada::Falhou));
}

#[test]
fn tres_erros_bloqueiam_mesmo_com_senha_certa() {
    let conn = cenario();
    let agora = Utc::now();
    for _ in 0..2 {
        assert!(matches!(funcionarios::entrar(&conn, "ana", "errada", agora).unwrap(), Entrada::Falhou));
    }
    let terceiro = funcionarios::entrar(&conn, "ana", "errada", agora).unwrap();
    assert!(matches!(terceiro, Entrada::Bloqueado { .. }), "o terceiro erro bloqueia");
    // Bloqueado: nem a senha certa entra antes do prazo.
    assert!(matches!(funcionarios::entrar(&conn, "ana", "senha12345", agora + Duration::minutes(1)).unwrap(), Entrada::Bloqueado { .. }));
    // Depois do prazo volta a aceitar a senha certa.
    assert!(matches!(funcionarios::entrar(&conn, "ana", "senha12345", agora + Duration::minutes(6)).unwrap(), Entrada::Ok(_)));
}

#[test]
fn login_desconhecido_nao_bloqueia_nem_revela() {
    let conn = cenario();
    let agora = Utc::now();
    for _ in 0..5 {
        assert!(matches!(funcionarios::entrar(&conn, "ninguem", "x", agora).unwrap(), Entrada::Falhou));
    }
}
