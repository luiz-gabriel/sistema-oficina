// Regras de sincronização: snapshot fiel, conflito por registro alterado e remapeamento de ids locais.
use oficina_core::comandos::{self, Contexto};
use oficina_core::db;
use oficina_core::sync;
use serde_json::{json, Value};

fn admin() -> Contexto {
    Contexto { usuario: "admin".into(), perfil: "administrador".into(), computador: "teste".into() }
}

fn cria_cliente(conn: &rusqlite::Connection, nome: &str) -> i64 {
    let v = comandos::executar(conn, "clientes_criar", &json!({"dados": {"nome": nome}}), &admin()).unwrap();
    v["id"].as_i64().unwrap()
}

#[test]
fn snapshot_copia_todos_os_dados_para_outro_banco() {
    let a = db::open_in_memory().unwrap();
    let cli = cria_cliente(&a, "Ana");
    comandos::executar(&a, "veiculos_criar", &json!({"dados": {"cliente_id": cli, "placa": "ABC1D23", "modelo": "Gol", "motor": "1.6"}}), &admin()).unwrap();
    comandos::executar(&a, "itens_criar", &json!({"dados": {"nome": "Filtro", "tipo": "peca", "preco_venda": 35.5, "quantidade": 2}}), &admin()).unwrap();

    let mut b = db::open_in_memory().unwrap();
    let snap = sync::snapshot(&a).unwrap();
    sync::aplicar_snapshot(&mut b, &snap).unwrap();
    assert_eq!(sync::snapshot(&b).unwrap(), snap);
}

#[test]
fn conflito_quando_o_servidor_mudou_o_registro() {
    let servidor = db::open_in_memory().unwrap();
    let id = cria_cliente(&servidor, "Ana");
    // Cliente copia o registro e fica offline
    let base = sync::hash_linha(&sync::linha_json(&servidor, "clientes", id).unwrap().unwrap());
    // Outro computador altera o registro no servidor
    comandos::executar(&servidor, "clientes_atualizar", &json!({"id": id, "dados": {"nome": "Ana Maria"}}), &admin()).unwrap();
    // A alteração offline chega com a base antiga: deve ser recusada como conflito
    let conflito = comandos::verificar_base(&servidor, "clientes_atualizar", &json!({"id": id, "dados": {"nome": "Ana Paula"}}), &base).unwrap();
    let atual = conflito.expect("deveria apontar conflito");
    assert_eq!(atual["nome"], json!("Ana Maria"));
}

#[test]
fn sem_mudanca_no_servidor_nao_ha_conflito() {
    let servidor = db::open_in_memory().unwrap();
    let id = cria_cliente(&servidor, "Ana");
    let base = sync::hash_linha(&sync::linha_json(&servidor, "clientes", id).unwrap().unwrap());
    let r = comandos::verificar_base(&servidor, "clientes_atualizar", &json!({"id": id, "dados": {"nome": "Ana B"}}), &base).unwrap();
    assert!(r.is_none());
}

#[test]
fn remover_registro_ja_removido_no_servidor_e_conflito() {
    let servidor = db::open_in_memory().unwrap();
    let r = comandos::verificar_base(&servidor, "clientes_remover", &json!({"id": 99}), "qualquer").unwrap();
    assert_eq!(r, Some(Value::Null));
}

#[test]
fn ids_locais_sao_trocados_pelos_do_servidor_nas_referencias() {
    // Veículo criado offline referencia cliente criado offline: os dois ids locais viram ids do servidor.
    let args = json!({"dados": {"cliente_id": 5, "placa": "ABC1D23", "modelo": "Gol"}});
    let remapeado = comandos::remapear("veiculos_criar", &args, &|t, id| {
        Ok(if t == "clientes" && id == 5 { Some(42) } else { None })
    })
    .unwrap();
    assert_eq!(remapeado["dados"]["cliente_id"], json!(42));
    // Campos que não são referência ficam como estão
    assert_eq!(remapeado["dados"]["placa"], json!("ABC1D23"));
}

#[test]
fn id_do_proprio_registro_so_e_trocado_em_atualizacao() {
    let args = json!({"id": 7, "dados": {"nome": "X"}});
    let r = comandos::remapear("clientes_atualizar", &args, &|t, id| Ok(if t == "clientes" && id == 7 { Some(70) } else { None })).unwrap();
    assert_eq!(r["id"], json!(70));
    let criar = comandos::remapear("clientes_criar", &json!({"dados": {"nome": "Y"}}), &|_, _| Ok(Some(1))).unwrap();
    assert_eq!(criar, json!({"dados": {"nome": "Y"}}));
}

#[test]
fn funcionario_sem_acesso_nao_recebe_senha_e_admin_pode_conceder() {
    let c = db::open_in_memory().unwrap();
    let f = comandos::executar(&c, "funcionarios_criar", &json!({"dados": {"nome": "Carlos"}}), &admin()).unwrap();
    assert_eq!(f["tem_acesso"], json!(false));
    let id = f["id"].as_i64().unwrap();
    let senha_fraca = comandos::executar(&c, "funcionarios_dar_acesso", &json!({"id": id, "dados": {"login": "carlos", "senha": "123", "perfil": "funcionario"}}), &admin());
    assert!(senha_fraca.is_err());
    let ok = comandos::executar(&c, "funcionarios_dar_acesso", &json!({"id": id, "dados": {"login": "carlos", "senha": "oficina2026", "perfil": "funcionario"}}), &admin()).unwrap();
    assert_eq!(ok["tem_acesso"], json!(true));
    assert!(oficina_core::funcionarios::autenticar(&c, "carlos", "oficina2026").unwrap().is_some());
    assert!(oficina_core::funcionarios::autenticar(&c, "carlos", "errada2026").unwrap().is_none());
}

#[test]
fn funcionario_comum_nao_cadastra_nem_concede_acesso() {
    // Cadastro e acesso de funcionários são só do administrador.
    let c = db::open_in_memory().unwrap();
    let comum = Contexto { usuario: "x".into(), perfil: "funcionario".into(), computador: "t".into() };
    assert!(comandos::executar(&c, "funcionarios_criar", &json!({"dados": {"nome": "Y"}}), &comum).is_err());
    let adm = Contexto { usuario: "adm".into(), perfil: "administrador".into(), computador: "t".into() };
    let f = comandos::executar(&c, "funcionarios_criar", &json!({"dados": {"nome": "Y"}}), &adm).unwrap();
    let id = f["id"].as_i64().unwrap();
    assert!(comandos::executar(&c, "funcionarios_dar_acesso", &json!({"id": id, "dados": {"login": "y", "senha": "Oficina2026x", "perfil": "administrador"}}), &comum).is_err());
}

#[test]
fn ativacao_so_aceita_codigo_da_lista_e_so_uma_vez() {
    let c = db::open_in_memory().unwrap();
    assert!(oficina_core::ativacao::ativar(&c, "AAAA-AAAA-AAAA-AAAA").is_err());
    let codigo = oficina_core::ativacao::codigos_validos()[0].clone();
    assert_eq!(oficina_core::ativacao::codigos_validos().len(), 50);
    oficina_core::ativacao::ativar(&c, &codigo.to_lowercase()).unwrap();
    assert!(oficina_core::ativacao::esta_ativado(&c).unwrap());
    assert!(oficina_core::ativacao::ativar(&c, &codigo).is_err());
}
