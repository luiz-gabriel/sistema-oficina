// Sessões de login persistidas: validade absoluta de 10 horas, sem renovação por uso, sobrevivem a reinício.
use std::path::PathBuf;

use chrono::{DateTime, Duration, TimeZone, Utc};
use oficina_core::dispositivos;
use oficina_core::funcionarios::{self, AcessoInput, FuncionarioInput};
use oficina_core::seguranca::sha256_hex;
use oficina_core::sessoes::{self, SessaoAtiva, VALIDADE_SEGUNDOS};
use oficina_core::{db, rede_regras};
use rusqlite::Connection;

fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 23, 8, 0, 0).unwrap()
}

/// Banco com o computador principal (aparelho ATIVO), que é onde as sessões ficam ligadas.
fn banco_com_principal(conn: &Connection) -> i64 {
    dispositivos::garantir_principal(conn, "PC", t0()).unwrap()
}

fn caminho_banco() -> PathBuf {
    std::env::temp_dir().join(format!("oficina_sessao_{}.db", uuid::Uuid::new_v4()))
}

fn valida(conn: &Connection, token: &str, agora: DateTime<Utc>) -> Option<SessaoAtiva> {
    sessoes::validar(conn, token, agora).unwrap()
}

#[test]
fn validade_e_de_dez_horas_absolutas() {
    assert_eq!(VALIDADE_SEGUNDOS, 36_000);
    // A validade do login de rede também passou para 10 horas.
    assert_eq!(rede_regras::VALIDADE_SESSAO, std::time::Duration::from_secs(36_000));
    let conn = db::open_in_memory().unwrap();
    let dev = banco_com_principal(&conn);
    let token = sessoes::criar(&conn, "ana", dev, t0()).unwrap();
    assert!(valida(&conn, &token, t0() + Duration::hours(10) - Duration::seconds(1)).is_some());
    assert!(valida(&conn, &token, t0() + Duration::hours(10)).is_none(), "no instante de 10 horas já venceu");
}

#[test]
fn uso_nao_estende_o_prazo() {
    let conn = db::open_in_memory().unwrap();
    let dev = banco_com_principal(&conn);
    let token = sessoes::criar(&conn, "ana", dev, t0()).unwrap();
    // Usar a sessão o tempo todo não muda o fim dela.
    for horas in [1, 3, 5, 7, 9] {
        let s = valida(&conn, &token, t0() + Duration::hours(horas)).expect("ainda válida");
        assert_eq!(s.expira_em, (t0() + Duration::hours(10)).timestamp());
    }
    assert!(valida(&conn, &token, t0() + Duration::hours(10)).is_none());
    assert!(valida(&conn, &token, t0() + Duration::hours(10) + Duration::seconds(1)).is_none());
}

#[test]
fn restante_expoe_o_tempo_para_o_aviso_de_cinco_minutos() {
    let conn = db::open_in_memory().unwrap();
    let dev = banco_com_principal(&conn);
    let token = sessoes::criar(&conn, "ana", dev, t0()).unwrap();
    let s = valida(&conn, &token, t0() + Duration::hours(9)).unwrap();
    assert_eq!(sessoes::segundos_restantes(&s, t0() + Duration::hours(9)), 3600);
    let aviso = t0() + Duration::hours(10) - Duration::minutes(5);
    let s = valida(&conn, &token, aviso).unwrap();
    assert_eq!(sessoes::segundos_restantes(&s, aviso), 300);
    // Nunca negativo, mesmo se consultado depois do fim.
    let fim = t0() + Duration::hours(10) + Duration::seconds(30);
    assert_eq!(sessoes::segundos_restantes(&s, fim), 0);
}

#[test]
fn sessao_sobrevive_a_reinicio_sem_estender_o_prazo() {
    let caminho = caminho_banco();
    let token;
    {
        let conn = db::open(&caminho).unwrap();
        let dev = banco_com_principal(&conn);
        token = sessoes::criar(&conn, "ana", dev, t0()).unwrap();
    } // o servidor "reinicia": a conexão é fechada e o banco é aberto de novo
    let conn = db::open(&caminho).unwrap();
    let agora = t0() + Duration::hours(9);
    let s = valida(&conn, &token, agora).expect("continua válida depois do reinício");
    assert_eq!(s.usuario, "ana");
    assert_eq!(sessoes::segundos_restantes(&s, agora), 3600, "o prazo original continua valendo");
    // Reabrir não renova: passada a hora original, a sessão está vencida.
    assert!(valida(&conn, &token, t0() + Duration::hours(10)).is_none());
    drop(conn);
    let _ = std::fs::remove_file(&caminho);
}

#[test]
fn logout_encerra_imediatamente() {
    let conn = db::open_in_memory().unwrap();
    let dev = banco_com_principal(&conn);
    let token = sessoes::criar(&conn, "ana", dev, t0()).unwrap();
    assert!(valida(&conn, &token, t0()).is_some());
    sessoes::encerrar(&conn, &token).unwrap();
    assert!(valida(&conn, &token, t0()).is_none());
}

#[test]
fn token_nao_fica_em_claro_e_expiracao_e_inteiro() {
    let conn = db::open_in_memory().unwrap();
    let dev = banco_com_principal(&conn);
    let token = sessoes::criar(&conn, "ana", dev, t0()).unwrap();
    assert_eq!(token.len(), 64);
    let hash: String = conn.query_row("SELECT token_hash FROM sessoes", [], |r| r.get(0)).unwrap();
    assert_eq!(hash, sha256_hex(&token));
    let tipo: String = conn.query_row("SELECT typeof(expira_em) FROM sessoes", [], |r| r.get(0)).unwrap();
    assert_eq!(tipo, "integer");
}

#[test]
fn revogacao_recusada_mantem_a_sessao() {
    let conn = db::open_in_memory().unwrap();
    let principal = banco_com_principal(&conn);
    let f = funcionarios::criar(&conn, &FuncionarioInput { nome: Some("Carlos".into()), ..Default::default() }).unwrap();
    funcionarios::dar_acesso(&conn, f.id, &AcessoInput {
        login: Some("carlos".into()),
        senha: Some("senhaforte1".into()),
        senha_hash: None,
        perfil: Some("administrador".into()),
    })
    .unwrap();
    let token = sessoes::criar(&conn, "carlos", principal, t0()).unwrap();
    assert!(valida(&conn, &token, t0()).is_some());
    // Revogar o principal exige outro aparelho autorizado; sem isso a revogação é recusada e a sessão segue.
    assert!(dispositivos::revogar(&conn, principal).is_err());
    assert!(valida(&conn, &token, t0()).is_some());
}

#[test]
fn retirar_acesso_do_funcionario_encerra_as_sessoes_dele() {
    let conn = db::open_in_memory().unwrap();
    banco_com_principal(&conn);
    let carlos = funcionarios::criar(&conn, &FuncionarioInput { nome: Some("Carlos".into()), ..Default::default() }).unwrap();
    funcionarios::dar_acesso(&conn, carlos.id, &AcessoInput {
        login: Some("carlos".into()),
        senha: Some("senhaforte1".into()),
        senha_hash: None,
        perfil: Some("administrador".into()),
    })
    .unwrap();
    let ana = funcionarios::criar(&conn, &FuncionarioInput { nome: Some("Ana".into()), ..Default::default() }).unwrap();
    funcionarios::dar_acesso(&conn, ana.id, &AcessoInput {
        login: Some("ana".into()),
        senha: Some("senhaforte1".into()),
        senha_hash: None,
        perfil: Some("funcionario".into()),
    })
    .unwrap();
    let do_carlos = sessoes::criar(&conn, "carlos", 1, t0()).unwrap();
    let da_ana = sessoes::criar(&conn, "ana", 1, t0()).unwrap();
    // Ana também é administradora para que Carlos possa perder o acesso.
    funcionarios::dar_acesso(&conn, ana.id, &AcessoInput {
        login: Some("ana".into()),
        senha: Some("senhaforte1".into()),
        senha_hash: None,
        perfil: Some("administrador".into()),
    })
    .unwrap();
    funcionarios::revogar_acesso(&conn, carlos.id).unwrap();
    assert!(valida(&conn, &do_carlos, t0()).is_none(), "sessões do Carlos encerradas");
    assert!(valida(&conn, &da_ana, t0()).is_some(), "sessão da Ana não muda");
}

#[test]
fn sessao_vencida_e_apagada_na_consulta() {
    let conn = db::open_in_memory().unwrap();
    let dev = banco_com_principal(&conn);
    let token = sessoes::criar(&conn, "ana", dev, t0()).unwrap();
    assert!(valida(&conn, &token, t0() + Duration::hours(11)).is_none());
    let sobrando: i64 = conn.query_row("SELECT COUNT(*) FROM sessoes", [], |r| r.get(0)).unwrap();
    assert_eq!(sobrando, 0);
}
