//! Caixa por operador: quem abre fecha; funcionário não vê valores de outro caixa nem o financeiro.
use oficina_core::comandos::{executar, Contexto};
use oficina_core::db;
use rusqlite::Connection;
use serde_json::json;

fn conn() -> Connection {
    db::open_in_memory().unwrap()
}

fn ctx(usuario: &str, perfil: &str) -> Contexto {
    Contexto { usuario: usuario.into(), perfil: perfil.into(), computador: "PC1".into() }
}

#[test]
fn segundo_operador_nao_abre_enquanto_outro_tem_caixa_aberto() {
    let c = conn();
    executar(&c, "caixa_abrir", &json!({"valor_inicial": 50.0}), &ctx("ana", "funcionario")).unwrap();
    let erro = executar(&c, "caixa_abrir", &json!({"valor_inicial": 10.0}), &ctx("bruno", "funcionario")).unwrap_err();
    let texto = erro.to_string();
    assert!(texto.contains("aberto por ana"), "mensagem deve dizer de quem é: {texto}");
}

#[test]
fn funcionario_ve_caixa_de_outro_sem_valores() {
    let c = conn();
    executar(&c, "caixa_abrir", &json!({"valor_inicial": 50.0}), &ctx("ana", "administrador")).unwrap();
    let s = executar(&c, "caixa_situacao", &json!({}), &ctx("bruno", "funcionario")).unwrap();
    assert_eq!(s["de_outro_operador"], json!(true));
    assert_eq!(s["sessao"]["aberto_por"], json!("ana"));
    assert_eq!(s["dinheiro_esperado"], json!(0.0), "valores não podem aparecer para outro funcionário");
}

#[test]
fn funcionario_ve_o_proprio_caixa_com_valores() {
    let c = conn();
    executar(&c, "caixa_abrir", &json!({"valor_inicial": 50.0}), &ctx("ana", "funcionario")).unwrap();
    let s = executar(&c, "caixa_situacao", &json!({}), &ctx("ana", "funcionario")).unwrap();
    assert_eq!(s["de_outro_operador"], json!(false));
    assert_eq!(s["dinheiro_esperado"], json!(50.0));
}

#[test]
fn funcionario_nao_fecha_caixa_de_outro() {
    let c = conn();
    executar(&c, "caixa_abrir", &json!({"valor_inicial": 50.0}), &ctx("ana", "funcionario")).unwrap();
    let id = executar(&c, "caixa_situacao", &json!({}), &ctx("ana", "funcionario")).unwrap()["sessao"]["id"].clone();
    let erro = executar(&c, "caixa_fechar", &json!({"id": id, "valor_contado": 50.0}), &ctx("bruno", "funcionario"));
    assert!(erro.is_err(), "outro funcionário não pode fechar o caixa de Ana");
}

#[test]
fn administrador_fecha_caixa_de_outro_so_com_motivo() {
    let c = conn();
    executar(&c, "caixa_abrir", &json!({"valor_inicial": 50.0}), &ctx("ana", "funcionario")).unwrap();
    let id = executar(&c, "caixa_situacao", &json!({}), &ctx("ana", "funcionario")).unwrap()["sessao"]["id"].clone();
    let sem_motivo = executar(&c, "caixa_fechar", &json!({"id": id, "valor_contado": 50.0}), &ctx("gabriel", "administrador"));
    assert!(sem_motivo.is_err(), "admin precisa informar o motivo");
    let com_motivo = executar(&c, "caixa_fechar",
        &json!({"id": id, "valor_contado": 50.0, "observacao": "Ana saiu doente"}), &ctx("gabriel", "administrador"));
    assert!(com_motivo.is_ok());
}

#[test]
fn financeiro_e_relatorios_so_para_administrador() {
    let c = conn();
    for cmd in ["financeiro_resumo", "recebimentos_listar", "contas_receber_listar", "contas_pagar_listar", "relatorio_faturamento", "caixa_historico"] {
        let f = executar(&c, cmd, &json!({"de": "2026-01-01", "ate": "2026-12-31"}), &ctx("bruno", "funcionario"));
        assert!(f.is_err(), "{cmd} deve ser só do administrador");
        let a = executar(&c, cmd, &json!({"de": "2026-01-01", "ate": "2026-12-31"}), &ctx("gabriel", "administrador"));
        assert!(a.is_ok(), "{cmd} deve funcionar para o administrador: {:?}", a.err());
    }
}

#[test]
fn funcionario_nao_le_caixa_de_outro_pelo_id() {
    let c = conn();
    executar(&c, "caixa_abrir", &json!({"valor_inicial": 50.0}), &ctx("ana", "funcionario")).unwrap();
    let id = executar(&c, "caixa_situacao", &json!({}), &ctx("ana", "funcionario")).unwrap()["sessao"]["id"].clone();
    assert!(executar(&c, "caixa_relatorio", &json!({"id": id}), &ctx("bruno", "funcionario")).is_err());
    assert!(executar(&c, "caixa_relatorio", &json!({"id": id}), &ctx("ana", "funcionario")).is_ok());
}
