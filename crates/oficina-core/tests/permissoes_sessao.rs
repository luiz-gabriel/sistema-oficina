//! Permissões por perfil (funcionário x administrador) e sessão guardada para "Continuar como".
use chrono::{Duration, Utc};
use oficina_core::comandos::{self, pode_configurar, pode_executar, pode_ver_registro, Contexto};
use oficina_core::db;
use oficina_core::funcionarios::{self, AcessoInput, Entrada, FuncionarioInput};
use oficina_core::sessao;
use serde_json::json;

const SENHA: &str = "Segredo2026x";

fn ctx(perfil: &str) -> Contexto {
    Contexto { usuario: "teste".into(), perfil: perfil.into(), computador: "PC".into() }
}

fn com_admin_e_funcionario() -> (rusqlite::Connection, i64, i64) {
    let conn = db::open_in_memory().unwrap();
    let adm = funcionarios::criar(&conn, &FuncionarioInput { nome: Some("Carlos".into()), ..Default::default() }).unwrap();
    funcionarios::dar_acesso(&conn, adm.id, &AcessoInput { login: Some("carlos".into()), senha: Some(SENHA.into()), senha_hash: None, perfil: Some("administrador".into()) }).unwrap();
    let ana = funcionarios::criar(&conn, &FuncionarioInput { nome: Some("Ana".into()), ..Default::default() }).unwrap();
    funcionarios::dar_acesso(&conn, ana.id, &AcessoInput { login: Some("ana".into()), senha: Some(SENHA.into()), senha_hash: None, perfil: Some("funcionario".into()) }).unwrap();
    (conn, adm.id, ana.id)
}

// ---- Permissões por perfil ----

#[test]
fn administrador_pode_tudo() {
    let args = json!({"id": 1, "dados": {"login": "x", "perfil": "administrador"}});
    for cmd in ["funcionarios_dar_acesso", "funcionarios_revogar_acesso", "funcionarios_remover", "funcionarios_criar"] {
        assert!(pode_executar("administrador", cmd, &args, Some("administrador")), "{cmd}");
    }
}

#[test]
fn funcionario_nao_cadastra_nem_concede_acesso() {
    // Cadastro e acesso de funcionários são só do administrador.
    let args = json!({"id": 1, "dados": {"login": "x", "perfil": "funcionario"}});
    assert!(!pode_executar("funcionario", "funcionarios_criar", &json!({"dados": {"nome": "Y"}}), None));
    assert!(!pode_executar("funcionario", "funcionarios_atualizar", &json!({"id": 1, "dados": {"nome": "Y"}}), Some("funcionario")));
    assert!(!pode_executar("funcionario", "funcionarios_dar_acesso", &args, Some("funcionario")));
    assert!(!pode_executar("funcionario", "funcionarios_revogar_acesso", &json!({"id": 1}), Some("funcionario")));
    assert!(!pode_executar("funcionario", "funcionarios_remover", &json!({"id": 1}), Some("funcionario")));
    // Leitura da lista continua liberada (responsável na OS).
    assert!(pode_executar("funcionario", "funcionarios_listar", &json!({}), None));
    // Administrador pode tudo.
    assert!(pode_executar("administrador", "funcionarios_criar", &json!({"dados": {"nome": "Y"}}), None));
}

#[test]
fn funcionario_nao_concede_perfil_administrador() {
    let args = json!({"id": 1, "dados": {"login": "x", "perfil": "administrador"}});
    assert!(!pode_executar("funcionario", "funcionarios_dar_acesso", &args, Some("funcionario")));
}

#[test]
fn funcionario_nao_mexe_na_conta_de_administrador() {
    let args = json!({"id": 1, "dados": {"login": "x", "perfil": "funcionario"}});
    assert!(!pode_executar("funcionario", "funcionarios_dar_acesso", &args, Some("administrador")));
    assert!(!pode_executar("funcionario", "funcionarios_revogar_acesso", &json!({"id": 1}), Some("administrador")));
    assert!(!pode_executar("funcionario", "funcionarios_remover", &json!({"id": 1}), Some("administrador")));
    // Nem editar o cadastro de um administrador: cadastro de funcionário é só do administrador.
    assert!(!pode_executar("funcionario", "funcionarios_atualizar", &json!({"id": 1, "dados": {"nome": "Z"}}), Some("administrador")));
}

#[test]
fn perfil_desconhecido_tem_restricoes_de_funcionario() {
    let args = json!({"id": 1, "dados": {"perfil": "administrador"}});
    assert!(!pode_executar("gerente", "funcionarios_dar_acesso", &args, None));
    assert!(!pode_configurar("gerente"));
    assert!(!pode_ver_registro("gerente"));
}

#[test]
fn configuracao_e_registro_so_do_administrador() {
    assert!(pode_configurar("administrador"));
    assert!(pode_ver_registro("administrador"));
    assert!(!pode_configurar("funcionario"));
    assert!(!pode_ver_registro("funcionario"));
}

#[test]
fn banco_recusa_funcionario_que_tenta_virar_administrador() {
    let (conn, _adm, ana) = com_admin_e_funcionario();
    let comum = ctx("funcionario");
    let tentativa = json!({"id": ana, "dados": {"login": "ana", "senha": SENHA, "perfil": "administrador"}});
    assert!(comandos::executar(&conn, "funcionarios_dar_acesso", &tentativa, &comum).is_err());
    // Continua funcionário no banco.
    assert_eq!(funcionarios::buscar(&conn, ana).unwrap().perfil, "funcionario");
}

#[test]
fn banco_recusa_funcionario_que_mexe_no_administrador() {
    let (conn, adm, _ana) = com_admin_e_funcionario();
    let comum = ctx("funcionario");
    let troca_senha = json!({"id": adm, "dados": {"login": "carlos", "senha": "Outra2026x", "perfil": "administrador"}});
    assert!(comandos::executar(&conn, "funcionarios_dar_acesso", &troca_senha, &comum).is_err());
    assert!(comandos::executar(&conn, "funcionarios_revogar_acesso", &json!({"id": adm}), &comum).is_err());
    assert!(comandos::executar(&conn, "funcionarios_remover", &json!({"id": adm}), &comum).is_err());
    assert!(funcionarios::autenticar(&conn, "carlos", SENHA).unwrap().is_some());
}

#[test]
fn banco_recusa_cadastro_de_funcionario_por_funcionario_comum() {
    let (conn, _adm, _ana) = com_admin_e_funcionario();
    let comum = ctx("funcionario");
    let r = comandos::executar(&conn, "funcionarios_criar", &json!({"dados": {"nome": "Bruno"}}), &comum);
    assert!(r.is_err(), "funcionário comum não pode cadastrar funcionário");
    let args = json!({"id": 1, "dados": {"login": "bruno", "senha": SENHA, "perfil": "funcionario"}});
    assert!(comandos::executar(&conn, "funcionarios_dar_acesso", &args, &comum).is_err());
}

// ---- Sessão guardada ("Continuar como") ----

#[test]
fn janela_de_duas_horas() {
    let agora = Utc::now();
    let em = |min: i64| (agora - Duration::minutes(min)).to_rfc3339();
    assert_eq!(sessao::minutos_na_janela(&em(0), agora), Some(0));
    assert_eq!(sessao::minutos_na_janela(&em(119), agora), Some(119));
    assert_eq!(sessao::minutos_na_janela(&em(120), agora), Some(120));
    assert_eq!(sessao::minutos_na_janela(&em(121), agora), None);
    // Horário no futuro (relógio mudou): não oferece.
    assert_eq!(sessao::minutos_na_janela(&(agora + Duration::minutes(5)).to_rfc3339(), agora), None);
    assert_eq!(sessao::minutos_na_janela("não é data", agora), None);
}

#[test]
fn sessao_guardada_nao_tem_senha_nem_token() {
    let (conn, _adm, ana) = com_admin_e_funcionario();
    let f = match funcionarios::entrar(&conn, "ana", SENHA, Utc::now()).unwrap() {
        Entrada::Ok(f) => f,
        _ => panic!("login deveria funcionar"),
    };
    assert_eq!(f.id, ana);
    sessao::guardar(&conn, &f, Utc::now()).unwrap();

    let bruto = oficina_core::config::obter(&conn, "ultima_sessao").unwrap().expect("sessão guardada");
    assert!(!bruto.contains(SENHA), "senha em texto puro no banco");
    assert!(!bruto.contains("argon2"), "hash de senha no banco");
    assert!(!bruto.to_lowercase().contains("token"), "token no banco");
    let campos: Vec<String> = serde_json::from_str::<serde_json::Value>(&bruto)
        .unwrap()
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    let mut campos = campos;
    campos.sort();
    assert_eq!(campos, vec!["em", "nome", "perfil", "usuario"]);
}

#[test]
fn sessao_guardada_se_le_e_se_apaga() {
    let (conn, _adm, _ana) = com_admin_e_funcionario();
    let f = match funcionarios::entrar(&conn, "ana", SENHA, Utc::now()).unwrap() {
        Entrada::Ok(f) => f,
        _ => panic!("login deveria funcionar"),
    };
    sessao::guardar(&conn, &f, Utc::now()).unwrap();
    let lida = sessao::ler(&conn).unwrap().expect("sessão");
    assert_eq!((lida.usuario.as_str(), lida.nome.as_str(), lida.perfil.as_str()), ("ana", "Ana", "funcionario"));

    let (quem, minutos) = sessao::para_continuar(&conn, Utc::now()).unwrap().expect("continuar");
    assert_eq!(quem.nome, "Ana");
    assert_eq!(minutos, 0);

    sessao::apagar(&conn).unwrap();
    assert!(sessao::ler(&conn).unwrap().is_none());
    assert!(sessao::para_continuar(&conn, Utc::now()).unwrap().is_none());
}

#[test]
fn sessao_expirada_ou_sem_acesso_nao_continua() {
    let (conn, adm, ana) = com_admin_e_funcionario();
    let f = match funcionarios::entrar(&conn, "carlos", SENHA, Utc::now()).unwrap() {
        Entrada::Ok(f) => f,
        _ => panic!("login deveria funcionar"),
    };
    // Login de três horas atrás: fora da janela.
    sessao::guardar(&conn, &f, Utc::now() - Duration::hours(3)).unwrap();
    assert!(sessao::para_continuar(&conn, Utc::now()).unwrap().is_none());

    // Login recente, mas o acesso foi tirado depois: não oferece.
    sessao::guardar(&conn, &f, Utc::now()).unwrap();
    // Sempre deve restar um administrador com acesso: promove a Ana antes de tirar o acesso do Carlos.
    funcionarios::dar_acesso(&conn, ana, &AcessoInput { login: Some("ana".into()), senha: Some(SENHA.into()), senha_hash: None, perfil: Some("administrador".into()) }).unwrap();
    funcionarios::revogar_acesso(&conn, adm).unwrap();
    assert!(sessao::para_continuar(&conn, Utc::now()).unwrap().is_none());
}

#[test]
fn bloqueio_de_tentativas_vale_na_reentrada() {
    let (conn, _adm, _ana) = com_admin_e_funcionario();
    let agora = Utc::now();
    for _ in 0..3 {
        let _ = funcionarios::entrar(&conn, "ana", "errada9999", agora).unwrap();
    }
    // Mesmo com a senha certa, a reentrada segue bloqueada.
    assert!(matches!(funcionarios::entrar(&conn, "ana", SENHA, agora).unwrap(), Entrada::Bloqueado { .. }));
}
