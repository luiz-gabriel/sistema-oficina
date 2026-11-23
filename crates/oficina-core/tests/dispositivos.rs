// Dispositivos autorizados: limite de 10, código de pareamento, aprovação concorrente, revogação e administração.
use std::path::PathBuf;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, TimeZone, Utc};
use oficina_core::comandos::{self, Contexto};
use oficina_core::dispositivos::{self, Consulta, LIMITE_DISPOSITIVOS};
use oficina_core::error::AppError;
use oficina_core::funcionarios::{self, AcessoInput, FuncionarioInput};
use oficina_core::pareamento_pake;
use oficina_core::seguranca::sha256_hex;
use oficina_core::{db, sessoes};
use rusqlite::Connection;
use serde_json::json;

fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 23, 8, 0, 0).unwrap()
}

fn caminho_banco() -> PathBuf {
    std::env::temp_dir().join(format!("oficina_disp_{}.db", uuid::Uuid::new_v4()))
}

/// Cria um pedido PENDENTE: inicia o pareamento e o aparelho pareia por SPAKE2 com o código.
/// Devolve o pedido e a prova de consulta (só vale com a chave da sessão).
fn pedido(conn: &Connection, nome: &str, agora: DateTime<Utc>) -> (i64, String) {
    let c = dispositivos::iniciar_pareamento(conn, agora).unwrap();
    let p = dispositivos::parear_aparelho(conn, &c.codigo, nome, agora).unwrap();
    (p.pedido_id, pareamento_pake::provar_consulta(&p.chave, p.pedido_id))
}

/// Decifra o token entregue usando a chave de transporte guardada no pedido.
fn token_de(conn: &Connection, pedido_id: i64, cifrado: &str) -> String {
    let chave_hex: String = conn
        .query_row("SELECT chave_transporte FROM pedidos_pareamento WHERE id = ?1", [pedido_id], |r| r.get(0))
        .unwrap();
    let chave = hex::decode(chave_hex).unwrap();
    pareamento_pake::decifrar_token(&chave, cifrado).unwrap()
}

fn contar_ativos(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM dispositivos WHERE situacao = 'ATIVO'", [], |r| r.get(0)).unwrap()
}

fn admin(conn: &Connection, login: &str) -> i64 {
    let f = funcionarios::criar(conn, &FuncionarioInput { nome: Some(login.into()), ..Default::default() }).unwrap();
    funcionarios::dar_acesso(conn, f.id, &AcessoInput {
        login: Some(login.into()),
        senha: Some("senhaforte1".into()),
        senha_hash: None,
        perfil: Some("administrador".into()),
    })
    .unwrap();
    f.id
}

#[test]
fn dez_dispositivos_aceitos_o_decimo_primeiro_recusado() {
    let conn = db::open_in_memory().unwrap();
    let agora = t0();
    dispositivos::garantir_principal(&conn, "PC Principal", agora).unwrap();
    // O principal conta: 1 + 9 aparelhos = 10 vagas.
    for i in 1..=9 {
        let (id, _) = pedido(&conn, &format!("Aparelho {i}"), agora);
        dispositivos::aprovar_pedido(&conn, id, "PC Principal", agora).unwrap();
    }
    assert_eq!(contar_ativos(&conn), LIMITE_DISPOSITIVOS);
    let (id, _) = pedido(&conn, "Aparelho 10", agora);
    let erro = dispositivos::aprovar_pedido(&conn, id, "PC Principal", agora).unwrap_err();
    assert!(matches!(erro, AppError::Conflict(_)), "{erro:?}");
    assert_eq!(contar_ativos(&conn), LIMITE_DISPOSITIVOS);
    // O pedido recusado pelo limite continua pendente: nada foi gravado pela metade.
    assert_eq!(dispositivos::listar_pedidos_pendentes(&conn).unwrap().len(), 1);
}

#[test]
fn vinte_aprovacoes_simultaneas_respeitam_o_limite() {
    let caminho = caminho_banco();
    let principal = db::open(&caminho).unwrap();
    let agora = t0();
    dispositivos::garantir_principal(&principal, "PC Principal", agora).unwrap();
    let mut ids = Vec::new();
    for i in 0..20 {
        ids.push(pedido(&principal, &format!("Aparelho {i}"), agora).0);
    }
    // Uma conexão separada por thread, todas abertas antes de começar.
    let mut conexoes = Vec::new();
    for _ in 0..20 {
        let c = db::open(&caminho).unwrap();
        c.busy_timeout(StdDuration::from_secs(30)).unwrap();
        conexoes.push(c);
    }
    let barreira = Arc::new(Barrier::new(20));
    let handles: Vec<_> = conexoes
        .into_iter()
        .zip(ids)
        .map(|(c, id)| {
            let barreira = barreira.clone();
            thread::spawn(move || {
                barreira.wait();
                dispositivos::aprovar_pedido(&c, id, "PC Principal", agora)
            })
        })
        .collect();
    let mut aceitos = 0;
    let mut recusados = 0;
    for h in handles {
        match h.join().unwrap() {
            Ok(_) => aceitos += 1,
            Err(AppError::Conflict(_)) => recusados += 1,
            Err(outro) => panic!("erro inesperado (possível concorrência mal tratada): {outro:?}"),
        }
    }
    assert_eq!(aceitos, 9, "principal + 9 aparelhos = 10");
    assert_eq!(recusados, 11);
    assert_eq!(contar_ativos(&principal), LIMITE_DISPOSITIVOS);
    drop(principal);
    let _ = std::fs::remove_file(&caminho);
}

#[test]
fn codigo_tem_seis_digitos_e_nao_fica_em_claro() {
    let conn = db::open_in_memory().unwrap();
    let c = dispositivos::iniciar_pareamento(&conn, t0()).unwrap();
    assert_eq!(c.codigo.len(), 6);
    assert!(c.codigo.chars().all(|ch| ch.is_ascii_digit()));
    assert_eq!(c.expira_em, t0().timestamp() + 300);
    let gravados: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM codigos_pareamento WHERE codigo_hash = ?1 OR sal = ?1",
            [&c.codigo],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(gravados, 0, "o código não pode aparecer no banco");
}

#[test]
fn codigo_expira_em_cinco_minutos() {
    let conn = db::open_in_memory().unwrap();
    let c = dispositivos::iniciar_pareamento(&conn, t0()).unwrap();
    let no_limite = t0() + Duration::minutes(5);
    let erro = dispositivos::parear_aparelho(&conn, &c.codigo, "Celular", no_limite).unwrap_err();
    assert!(matches!(erro, AppError::Validation(_)), "{erro:?}");
    assert!(dispositivos::listar_pedidos_pendentes(&conn).unwrap().is_empty());
}

#[test]
fn codigo_ainda_vale_um_segundo_antes_de_expirar() {
    let conn = db::open_in_memory().unwrap();
    let c = dispositivos::iniciar_pareamento(&conn, t0()).unwrap();
    let antes = t0() + Duration::minutes(5) - Duration::seconds(1);
    assert!(dispositivos::parear_aparelho(&conn, &c.codigo, "Celular", antes).is_ok());
}

#[test]
fn codigo_usado_nao_serve_de_novo() {
    let conn = db::open_in_memory().unwrap();
    let agora = t0();
    let c = dispositivos::iniciar_pareamento(&conn, agora).unwrap();
    dispositivos::parear_aparelho(&conn, &c.codigo, "Celular da Ana", agora).unwrap();
    let de_novo = dispositivos::parear_aparelho(&conn, &c.codigo, "Outro aparelho", agora).unwrap_err();
    assert!(matches!(de_novo, AppError::Validation(_)), "{de_novo:?}");
    assert_eq!(dispositivos::listar_pedidos_pendentes(&conn).unwrap().len(), 1);
}

#[test]
fn tres_erradas_bloqueiam_o_codigo() {
    let conn = db::open_in_memory().unwrap();
    let agora = t0();
    let c = dispositivos::iniciar_pareamento(&conn, agora).unwrap();
    let errado = if c.codigo == "000000" { "111111" } else { "000000" };
    let primeira = dispositivos::parear_aparelho(&conn, errado, "Celular", agora).unwrap_err();
    assert!(format!("{primeira:?}").contains("Restam 2"), "{primeira:?}");
    dispositivos::parear_aparelho(&conn, errado, "Celular", agora).unwrap_err();
    let terceira = dispositivos::parear_aparelho(&conn, errado, "Celular", agora).unwrap_err();
    assert!(format!("{terceira:?}").contains("bloqueado"), "{terceira:?}");
    // Depois de bloqueado, nem o código certo serve.
    let certo = dispositivos::parear_aparelho(&conn, &c.codigo, "Celular", agora).unwrap_err();
    assert!(matches!(certo, AppError::Validation(_)), "{certo:?}");
    assert!(dispositivos::listar_pedidos_pendentes(&conn).unwrap().is_empty());
}

#[test]
fn duas_erradas_ainda_deixam_acertar() {
    let conn = db::open_in_memory().unwrap();
    let agora = t0();
    let c = dispositivos::iniciar_pareamento(&conn, agora).unwrap();
    let errado = if c.codigo == "000000" { "111111" } else { "000000" };
    dispositivos::parear_aparelho(&conn, errado, "Celular", agora).unwrap_err();
    dispositivos::parear_aparelho(&conn, errado, "Celular", agora).unwrap_err();
    assert!(dispositivos::parear_aparelho(&conn, &c.codigo, "Celular", agora).is_ok());
}

#[test]
fn novo_codigo_substitui_o_anterior() {
    let conn = db::open_in_memory().unwrap();
    let agora = t0();
    let antigo = dispositivos::iniciar_pareamento(&conn, agora).unwrap();
    // Garante códigos diferentes (o sorteio pode repetir); o antigo tem de deixar de valer.
    let novo = loop {
        let n = dispositivos::iniciar_pareamento(&conn, agora).unwrap();
        if n.codigo != antigo.codigo {
            break n;
        }
    };
    assert!(dispositivos::parear_aparelho(&conn, &antigo.codigo, "Celular", agora).is_err());
    assert!(dispositivos::parear_aparelho(&conn, &novo.codigo, "Celular", agora).is_ok());
}

#[test]
fn recusar_nao_ocupa_vaga_e_o_aparelho_ve_recusado() {
    let conn = db::open_in_memory().unwrap();
    let agora = t0();
    dispositivos::garantir_principal(&conn, "PC", agora).unwrap();
    let (id, segredo) = pedido(&conn, "Tablet", agora);
    dispositivos::recusar_pedido(&conn, id, agora).unwrap();
    assert_eq!(contar_ativos(&conn), 1);
    assert_eq!(dispositivos::consultar_pedido(&conn, id, &segredo, agora).unwrap(), Consulta::Recusado);
    assert!(dispositivos::aprovar_pedido(&conn, id, "PC", agora).is_err(), "pedido já decidido");
}

#[test]
fn token_entregue_uma_vez_e_so_o_hash_fica_no_banco() {
    let conn = db::open_in_memory().unwrap();
    let agora = t0();
    dispositivos::garantir_principal(&conn, "PC", agora).unwrap();
    let (id, segredo) = pedido(&conn, "Celular", agora);
    let d = dispositivos::aprovar_pedido(&conn, id, "PC", agora).unwrap();
    assert!(!d.principal);
    let token = match dispositivos::consultar_pedido(&conn, id, &segredo, agora).unwrap() {
        Consulta::Liberado { token_cifrado } => token_de(&conn, id, &token_cifrado),
        outra => panic!("esperava o token, veio {outra:?}"),
    };
    assert_eq!(token.len(), 64);
    assert_eq!(dispositivos::consultar_pedido(&conn, id, &segredo, agora).unwrap(), Consulta::JaEntregue);
    let guardado: String = conn
        .query_row("SELECT credencial_hash FROM dispositivos WHERE id = ?1", [d.id], |r| r.get(0))
        .unwrap();
    assert_eq!(guardado, sha256_hex(&token));
    assert_ne!(guardado, token);
    assert_eq!(dispositivos::autenticar_dispositivo(&conn, &token, agora).unwrap(), Some(d.id));
    assert_eq!(dispositivos::autenticar_dispositivo(&conn, "token-falso", agora).unwrap(), None);
}

#[test]
fn segredo_errado_nao_consulta_o_pedido() {
    let conn = db::open_in_memory().unwrap();
    let agora = t0();
    dispositivos::garantir_principal(&conn, "PC", agora).unwrap();
    let (id, _segredo) = pedido(&conn, "Celular", agora);
    dispositivos::aprovar_pedido(&conn, id, "PC", agora).unwrap();
    let erro = dispositivos::consultar_pedido(&conn, id, "outro-segredo", agora).unwrap_err();
    assert!(matches!(erro, AppError::NotFound(_)), "{erro:?}");
}

#[test]
fn revogacao_libera_vaga_e_invalida_sessoes_do_aparelho() {
    let conn = db::open_in_memory().unwrap();
    let agora = t0();
    admin(&conn, "carlos");
    dispositivos::garantir_principal(&conn, "PC", agora).unwrap();
    let mut ids = Vec::new();
    for i in 1..=9 {
        let (pid, segredo) = pedido(&conn, &format!("Aparelho {i}"), agora);
        let d = dispositivos::aprovar_pedido(&conn, pid, "PC", agora).unwrap();
        let token = match dispositivos::consultar_pedido(&conn, pid, &segredo, agora).unwrap() {
            Consulta::Liberado { token_cifrado } => token_de(&conn, pid, &token_cifrado),
            outra => panic!("{outra:?}"),
        };
        ids.push((d.id, token));
    }
    assert_eq!(contar_ativos(&conn), LIMITE_DISPOSITIVOS);
    let (aparelho, token) = ids[0].clone();
    let sessao_token = sessoes::criar(&conn, "carlos", aparelho, agora).unwrap();
    assert!(sessoes::validar(&conn, &sessao_token, agora).unwrap().is_some());

    dispositivos::revogar(&conn, aparelho).unwrap();

    assert_eq!(contar_ativos(&conn), LIMITE_DISPOSITIVOS - 1, "a vaga volta");
    assert!(sessoes::validar(&conn, &sessao_token, agora).unwrap().is_none(), "sessão do aparelho invalidada");
    assert_eq!(dispositivos::autenticar_dispositivo(&conn, &token, agora).unwrap(), None);
    let sobrando: i64 = conn
        .query_row("SELECT COUNT(*) FROM sessoes WHERE dispositivo_id = ?1", [aparelho], |r| r.get(0))
        .unwrap();
    assert_eq!(sobrando, 0);
    // A vaga liberada pode ser ocupada de novo.
    let (pid, _) = pedido(&conn, "Novo aparelho", agora);
    dispositivos::aprovar_pedido(&conn, pid, "PC", agora).unwrap();
    assert_eq!(contar_ativos(&conn), LIMITE_DISPOSITIVOS);
    // Revogar de novo o mesmo aparelho é recusado.
    assert!(matches!(dispositivos::revogar(&conn, aparelho), Err(AppError::Conflict(_))));
}

#[test]
fn principal_so_e_revogado_se_restar_outra_administracao() {
    let conn = db::open_in_memory().unwrap();
    let agora = t0();
    admin(&conn, "carlos");
    let principal = dispositivos::garantir_principal(&conn, "PC", agora).unwrap();
    // Sem outro aparelho autorizado, o principal não sai: o sistema ficaria sem como administrar.
    let erro = dispositivos::revogar(&conn, principal).unwrap_err();
    assert!(matches!(erro, AppError::Conflict(_)), "{erro:?}");
    assert!(dispositivos::esta_ativo(&conn, principal).unwrap(), "recusa não deixa nada pela metade");

    let (pid, _) = pedido(&conn, "Celular", agora);
    let celular = dispositivos::aprovar_pedido(&conn, pid, "PC", agora).unwrap();
    dispositivos::revogar(&conn, principal).unwrap();
    assert!(!dispositivos::esta_ativo(&conn, principal).unwrap());
    // O principal revogado não entra de novo pelo login local.
    assert_eq!(dispositivos::garantir_principal(&conn, "PC", agora).unwrap(), principal);
    // Agora o último aparelho ativo não pode ser revogado: sem principal, ficaria sem administração.
    let erro = dispositivos::revogar(&conn, celular.id).unwrap_err();
    assert!(matches!(erro, AppError::Conflict(_)), "{erro:?}");
    assert!(dispositivos::esta_ativo(&conn, celular.id).unwrap());
}

#[test]
fn nao_revoga_o_ultimo_administrador_com_acesso() {
    let conn = db::open_in_memory().unwrap();
    let unico = admin(&conn, "carlos");
    let erro = funcionarios::revogar_acesso(&conn, unico).unwrap_err();
    assert!(matches!(erro, AppError::Conflict(_)), "{erro:?}");
    let erro = funcionarios::remover(&conn, unico).unwrap_err();
    assert!(matches!(erro, AppError::Conflict(_)), "{erro:?}");
    let outro = admin(&conn, "bia");
    funcionarios::revogar_acesso(&conn, unico).unwrap();
    assert!(funcionarios::existe_administrador(&conn).unwrap());
    let _ = outro;
}

#[test]
fn comandos_de_dispositivos_so_para_administrador() {
    let conn = db::open_in_memory().unwrap();
    let funcionario = Contexto { usuario: "ana".into(), perfil: "funcionario".into(), computador: "PC".into() };
    let adm = Contexto { usuario: "carlos".into(), perfil: "administrador".into(), computador: "PC".into() };
    assert!(comandos::executar(&conn, "dispositivos_listar", &json!({}), &funcionario).is_err());
    assert!(comandos::executar(&conn, "dispositivos_iniciar_pareamento", &json!({}), &funcionario).is_err());
    assert!(comandos::executar(&conn, "pareamento_listar_pedidos", &json!({}), &funcionario).is_err());
    // Sem nenhum uso anterior, a listagem já mostra o computador principal (nunca funcionário gera a linha).
    let lista = comandos::executar(&conn, "dispositivos_listar", &json!({}), &adm).unwrap();
    assert_eq!(lista.as_array().unwrap().len(), 1);
    assert_eq!(lista[0]["principal"], true);
    assert!(lista[0].get("credencial_hash").is_none(), "a lista não expõe o hash da credencial");
}

#[test]
fn principal_e_listado_primeiro_e_sem_segredos() {
    let conn = db::open_in_memory().unwrap();
    let agora = t0();
    dispositivos::garantir_principal(&conn, "PC", agora).unwrap();
    let (pid, _) = pedido(&conn, "Celular", agora);
    dispositivos::aprovar_pedido(&conn, pid, "PC", agora).unwrap();
    let lista = dispositivos::listar(&conn).unwrap();
    assert!(lista[0].principal);
    assert_eq!(lista[1].nome, "Celular");
    assert_eq!(lista[1].situacao, "ATIVO");
    assert!(lista[1].autorizado_em.is_some());
}

#[test]
fn sessao_de_pareamento_so_serve_uma_vez() {
    let conn = db::open_in_memory().unwrap();
    let agora = t0();
    dispositivos::garantir_principal(&conn, "PC", agora).unwrap();
    let c = dispositivos::iniciar_pareamento(&conn, agora).unwrap();
    let (iniciado, msg_c) = pareamento_pake::cliente_iniciar(&c.codigo);
    let sessao = dispositivos::iniciar_sessao(&conn, &msg_c, agora).unwrap();
    let chave = pareamento_pake::cliente_concluir(iniciado, &sessao.msg_servidor).unwrap();
    let mac = pareamento_pake::confirmar_cliente(&chave, &msg_c, &sessao.msg_servidor);
    dispositivos::confirmar_sessao(&conn, sessao.sessao_id, "Celular", &mac, None, agora).unwrap();
    let de_novo = dispositivos::confirmar_sessao(&conn, sessao.sessao_id, "Celular", &mac, None, agora).unwrap_err();
    assert!(matches!(de_novo, AppError::Validation(_)), "{de_novo:?}");
}

#[test]
fn mac_errado_gasta_tentativa_e_nao_cria_pedido() {
    let conn = db::open_in_memory().unwrap();
    let agora = t0();
    dispositivos::garantir_principal(&conn, "PC", agora).unwrap();
    let c = dispositivos::iniciar_pareamento(&conn, agora).unwrap();
    let (_iniciado, msg_c) = pareamento_pake::cliente_iniciar("000000");
    let sessao = dispositivos::iniciar_sessao(&conn, &msg_c, agora).unwrap();
    let falso = vec![0u8; 32];
    let erro = dispositivos::confirmar_sessao(&conn, sessao.sessao_id, "Celular", &falso, None, agora).unwrap_err();
    let AppError::Validation(campos) = &erro else { panic!("{erro:?}") };
    assert!(campos[0].message.contains("Restam 2"), "{erro:?}");
    let pedidos: i64 = conn.query_row("SELECT COUNT(*) FROM pedidos_pareamento", [], |r| r.get(0)).unwrap();
    assert_eq!(pedidos, 0);
    let erradas: i64 = conn
        .query_row("SELECT tentativas_erradas FROM codigos_pareamento ORDER BY id DESC LIMIT 1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(erradas, 1, "o código {} segue ativo, com uma tentativa gasta", c.codigo);
}

#[test]
fn codigo_em_claro_some_do_banco_depois_de_usado() {
    let conn = db::open_in_memory().unwrap();
    let agora = t0();
    dispositivos::garantir_principal(&conn, "PC", agora).unwrap();
    let c = dispositivos::iniciar_pareamento(&conn, agora).unwrap();
    let guardado: Option<String> = conn
        .query_row("SELECT codigo_pake FROM codigos_pareamento ORDER BY id DESC LIMIT 1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(guardado.as_deref(), Some(c.codigo.as_str()), "enquanto ativo, o SPAKE2 precisa do código");
    dispositivos::parear_aparelho(&conn, &c.codigo, "Celular", agora).unwrap();
    let depois: Option<String> = conn
        .query_row("SELECT codigo_pake FROM codigos_pareamento ORDER BY id DESC LIMIT 1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(depois, None);
}

#[test]
fn sessao_de_codigo_substituido_nao_confirma() {
    let conn = db::open_in_memory().unwrap();
    let agora = t0();
    dispositivos::garantir_principal(&conn, "PC", agora).unwrap();
    let antigo = dispositivos::iniciar_pareamento(&conn, agora).unwrap();
    let (iniciado, msg_c) = pareamento_pake::cliente_iniciar(&antigo.codigo);
    let sessao = dispositivos::iniciar_sessao(&conn, &msg_c, agora).unwrap();
    let chave = pareamento_pake::cliente_concluir(iniciado, &sessao.msg_servidor).unwrap();
    let mac = pareamento_pake::confirmar_cliente(&chave, &msg_c, &sessao.msg_servidor);
    dispositivos::iniciar_pareamento(&conn, agora).unwrap();
    assert!(dispositivos::confirmar_sessao(&conn, sessao.sessao_id, "Celular", &mac, None, agora).is_err());
}

#[test]
fn consulta_sem_prova_valida_nao_mostra_o_pedido() {
    let conn = db::open_in_memory().unwrap();
    let agora = t0();
    dispositivos::garantir_principal(&conn, "PC", agora).unwrap();
    let (id, _prova) = pedido(&conn, "Celular", agora);
    let outra_prova = pareamento_pake::provar_consulta(&[1u8; 32], id);
    assert!(dispositivos::consultar_pedido(&conn, id, &outra_prova, agora).is_err());
}

/// Pareia um aparelho informando a identidade do computador na rede local (ou nenhuma).
fn parear_com_identidade(conn: &Connection, identidade: Option<&str>, agora: DateTime<Utc>) -> i64 {
    let c = dispositivos::iniciar_pareamento(conn, agora).unwrap();
    let (iniciado, msg_c) = pareamento_pake::cliente_iniciar(&c.codigo);
    let sessao = dispositivos::iniciar_sessao(conn, &msg_c, agora).unwrap();
    let chave = pareamento_pake::cliente_concluir(iniciado, &sessao.msg_servidor).unwrap();
    let mac = pareamento_pake::confirmar_cliente(&chave, &msg_c, &sessao.msg_servidor);
    let confirmado = dispositivos::confirmar_sessao(conn, sessao.sessao_id, "PC da recepção", &mac, identidade, agora).unwrap();
    dispositivos::aprovar_pedido(conn, confirmado.pedido_id, "PC", agora).unwrap().id
}

#[test]
fn identidade_da_rede_fica_revogada_so_depois_de_remover_o_aparelho() {
    let conn = db::open_in_memory().unwrap();
    let agora = t0();
    admin(&conn, "carlos");
    dispositivos::garantir_principal(&conn, "PC", agora).unwrap();
    assert!(!dispositivos::identidade_rede_revogada(&conn, "pc-recepcao").unwrap(), "desconhecida não é revogada");
    let id = parear_com_identidade(&conn, Some("pc-recepcao"), agora);
    assert!(!dispositivos::identidade_rede_revogada(&conn, "pc-recepcao").unwrap(), "aparelho ativo segue valendo");
    dispositivos::revogar(&conn, id).unwrap();
    assert!(dispositivos::identidade_rede_revogada(&conn, "pc-recepcao").unwrap());
    // Pareando de novo a mesma identidade, ela volta a valer.
    parear_com_identidade(&conn, Some("pc-recepcao"), agora);
    assert!(!dispositivos::identidade_rede_revogada(&conn, "pc-recepcao").unwrap());
}
