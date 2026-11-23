// Servidor que recebeu uma alteração pendente (ex.: era cliente quando gravou) aplica a escolha "local" sozinho.
use oficina_core::clientes::{self, ClienteInput};
use oficina_core::{alteracoes, comandos::{self, Contexto}, db};
use serde_json::json;

#[test]
fn escolha_local_no_servidor_grava_e_zera_a_fila() {
    let conn = db::open_in_memory().unwrap();
    let cli = clientes::criar(&conn, &ClienteInput { nome: Some("Maria".into()), telefone: Some("11900000000".into()), ..Default::default() }).unwrap();
    let args = json!({"id": cli.id, "dados": {"nome": "Maria", "telefone": "11000000001"}});
    let uuid = alteracoes::registrar(&conn, &alteracoes::Nova {
        computador: "PC-CLIENTE", usuario: "gerente", comando: "clientes_atualizar", args: &args,
        tabela: Some("clientes"), id_local: Some(cli.id), base_hash: Some("hash-antigo"), status: "conflito",
    }).unwrap();
    // o usuário escolhe a versão local: fica pendente e forçada
    alteracoes::resolver(&conn, &uuid, "local").unwrap();
    // o mesmo passo que o servidor faz em resolver_conflito
    let item = alteracoes::listar(&conn, &["pendente"]).unwrap().into_iter().find(|a| a.uuid == uuid).unwrap();
    let ctx = Contexto { usuario: item.usuario.clone(), perfil: "administrador".into(), computador: "PC-SERVIDOR".into() };
    comandos::executar(&conn, &item.comando, &item.args, &ctx).unwrap();
    alteracoes::atualizar(&conn, &uuid, "sincronizado", None, None).unwrap();

    let depois = clientes::buscar(&conn, cli.id).unwrap();
    assert_eq!(depois.telefone.as_deref(), Some("11000000001"));
    assert!(alteracoes::listar(&conn, &["pendente", "conflito", "erro"]).unwrap().is_empty());
}
