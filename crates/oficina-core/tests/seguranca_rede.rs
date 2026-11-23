// Segurança da rede: acesso à cópia só com sessão de usuário, conflito com as duas versões lado a lado
// (sem senha) e eleição que avisa quando não há servidor seguro.
use std::time::{Duration, Instant};

use oficina_core::alteracoes::{self, Nova};
use oficina_core::db;
use oficina_core::error::AppError;
use oficina_core::rede_regras::{Membro, Sessoes, VALIDADE_SESSAO};
use oficina_core::sync::{self, SituacaoServidor};
use rusqlite::{params, Connection};
use serde_json::json;

const SENHA_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$sal$hash-secreto";

fn membro(dispositivo: &str, ativado_em: &str, papel: &str, epoca: u64) -> Membro {
    Membro { dispositivo: dispositivo.into(), ativado_em: ativado_em.into(), papel: papel.into(), epoca }
}

const JAN: &str = "2024-01-10T09:00:00+00:00";
const FEV: &str = "2024-02-10T09:00:00+00:00";
const MAR: &str = "2024-03-10T09:00:00+00:00";

// ---- 1. Acesso à cópia dos dados ----------------------------------------------------------

#[test]
fn dispositivo_autorizado_sem_login_nao_recebe_dados() {
    let sessoes = Sessoes::default();
    let agora = Instant::now();
    // Sem token (o computador não tem usuário logado): nada.
    let erro = sync::exigir_sessao(&sessoes, "", "dev-1", agora).unwrap_err();
    assert!(matches!(erro, AppError::Conflict(ref m) if m == sync::MSG_SEM_SESSAO));
    // Mesmo com um token de outro dispositivo da licença: nada.
    let token = sessoes.emitir("ana", "administrador", "dev-2", agora);
    assert!(sync::exigir_sessao(&sessoes, &token, "dev-1", agora).is_err());
}

#[test]
fn sessao_de_usuario_no_mesmo_dispositivo_libera_a_copia() {
    let sessoes = Sessoes::default();
    let agora = Instant::now();
    let token = sessoes.emitir("ana", "administrador", "dev-1", agora);
    let sessao = sync::exigir_sessao(&sessoes, &token, "dev-1", agora).expect("sessão válida");
    assert_eq!(sessao.usuario, "ana");
}

#[test]
fn sessao_expirada_ou_encerrada_nao_libera_a_copia() {
    let sessoes = Sessoes::default();
    let agora = Instant::now();
    let token = sessoes.emitir("ana", "administrador", "dev-1", agora);
    let depois = agora + VALIDADE_SESSAO + Duration::from_secs(1);
    assert!(sync::exigir_sessao(&sessoes, &token, "dev-1", depois).is_err());
    let outro = sessoes.emitir("bia", "funcionario", "dev-1", agora);
    sessoes.encerrar(&outro);
    assert!(sync::exigir_sessao(&sessoes, &outro, "dev-1", agora).is_err());
}

// ---- 2. Conflito: versão local e versão do servidor, lado a lado, sem senha -----------------

/// Fila com uma alteração em conflito: `args` é o que o usuário fez, `atual` é o registro no servidor.
fn fila_com_conflito(conn: &Connection, args: &serde_json::Value, atual: &serde_json::Value) -> String {
    let uuid = alteracoes::registrar(conn, &Nova {
        computador: "PC-CLIENTE", usuario: "gerente", comando: "clientes_atualizar", args,
        tabela: Some("clientes"), id_local: None, base_hash: Some("base-antiga"), status: "conflito",
    })
    .unwrap();
    conn.execute(
        "UPDATE alteracoes SET conflito = ?2 WHERE uuid = ?1",
        params![uuid, json!({"servidor": atual, "local": args, "comando": "clientes_atualizar"}).to_string()],
    )
    .unwrap();
    uuid
}

#[test]
fn conflito_mostra_as_duas_versoes_sem_senha() {
    let conn = db::open_in_memory().unwrap();
    let args = json!({"id": 1, "dados": {"nome": "Ana Local", "telefone": "11911111111", "senha_hash": SENHA_HASH}});
    let atual = json!({"id": 1, "nome": "Ana Servidor", "telefone": "11911111111", "senha_hash": SENHA_HASH});
    let uuid = fila_com_conflito(&conn, &args, &atual);

    let v = alteracoes::versoes_do_conflito(&conn, &uuid).unwrap();
    assert_eq!(v.local["nome"], json!("Ana Local"));
    assert_eq!(v.servidor["nome"], json!("Ana Servidor"));
    // Só o campo que difere aparece na comparação, com as duas versões.
    assert_eq!(v.diferencas.len(), 1);
    assert_eq!(v.diferencas[0].campo, "nome");
    assert_eq!(v.diferencas[0].local, json!("Ana Local"));
    assert_eq!(v.diferencas[0].servidor, json!("Ana Servidor"));
    // Nenhum segredo sai para a tela.
    let texto = serde_json::to_string(&v).unwrap();
    assert!(!texto.contains("senha"), "campo de senha exposto: {texto}");
    assert!(!texto.contains(SENHA_HASH), "hash de senha exposto: {texto}");
}

#[test]
fn conflito_com_registro_removido_no_servidor_mostra_ausencia() {
    let conn = db::open_in_memory().unwrap();
    let args = json!({"id": 7, "dados": {"nome": "Bia"}});
    let uuid = fila_com_conflito(&conn, &args, &serde_json::Value::Null);
    let v = alteracoes::versoes_do_conflito(&conn, &uuid).unwrap();
    assert!(v.servidor.is_null());
    assert!(v.diferencas.is_empty());
    assert_eq!(v.local["nome"], json!("Bia"));
}

#[test]
fn versoes_so_existem_para_conflito_pendente_de_escolha() {
    let conn = db::open_in_memory().unwrap();
    assert!(alteracoes::versoes_do_conflito(&conn, "inexistente").is_err());
    let uuid = alteracoes::registrar(&conn, &Nova {
        computador: "PC", usuario: "admin", comando: "clientes_criar", args: &json!({"dados": {"nome": "X"}}),
        tabela: Some("clientes"), id_local: None, base_hash: None, status: "pendente",
    })
    .unwrap();
    assert!(alteracoes::versoes_do_conflito(&conn, &uuid).is_err());
}

#[test]
fn escolha_servidor_descarta_a_local_e_local_reenvia_forcada() {
    let conn = db::open_in_memory().unwrap();
    let args = json!({"id": 1, "dados": {"nome": "Ana Local"}});
    let uuid = fila_com_conflito(&conn, &args, &json!({"id": 1, "nome": "Ana Servidor"}));
    alteracoes::resolver(&conn, &uuid, "servidor").unwrap();
    assert!(alteracoes::listar(&conn, &["pendente"]).unwrap().is_empty());

    let uuid2 = fila_com_conflito(&conn, &args, &json!({"id": 1, "nome": "Ana Servidor"}));
    alteracoes::resolver(&conn, &uuid2, "local").unwrap();
    let item = alteracoes::listar(&conn, &["pendente"]).unwrap().pop().unwrap();
    assert!(item.forcar);
    assert!(item.base_hash.is_none());
}

// ---- 6. Eleição: sem servidor seguro ------------------------------------------------------

#[test]
fn sem_quorum_a_situacao_e_sem_servidor_seguro() {
    // Conhecidos: 4. Este computador vê só mais um (2 de 4 não é maioria estrita).
    let eu = membro("dev-a", JAN, "sem_servidor", 0);
    let outro = membro("dev-b", FEV, "sem_servidor", 0);
    match sync::situacao_eleicao(&eu, &[outro], 4) {
        SituacaoServidor::SemServidorSeguro { motivo } => assert!(motivo.contains("quórum"), "motivo: {motivo}"),
        outra => panic!("esperava sem servidor seguro, veio {outra:?}"),
    }
}

#[test]
fn com_quorum_o_mais_antigo_se_promove() {
    let eu = membro("dev-a", JAN, "cliente", 0);
    let outros = [membro("dev-b", FEV, "cliente", 0), membro("dev-c", MAR, "cliente", 0)];
    assert_eq!(sync::situacao_eleicao(&eu, &outros, 3), SituacaoServidor::Promover);
}

#[test]
fn com_quorum_mas_outro_tem_prioridade_continua_sem_servidor_seguro() {
    let eu = membro("dev-b", FEV, "cliente", 0);
    let outros = [membro("dev-a", JAN, "cliente", 0), membro("dev-c", MAR, "cliente", 0)];
    assert!(matches!(sync::situacao_eleicao(&eu, &outros, 3), SituacaoServidor::SemServidorSeguro { .. }));
}

#[test]
fn servidor_respondendo_e_informado_como_servidor_ativo() {
    let eu = membro("dev-a", JAN, "cliente", 0);
    let outros = [membro("dev-s", FEV, "servidor", 2)];
    assert_eq!(
        sync::situacao_eleicao(&eu, &outros, 2),
        SituacaoServidor::ServidorAtivo { dispositivo: "dev-s".into() }
    );
}
