// Fila de alterações: senha nunca em texto puro, migração de filas antigas e cópia local que preserva o que está na fila.
use oficina_core::alteracoes::{self, Nova};
use oficina_core::comandos::{self, Contexto};
use oficina_core::{db, funcionarios, sync};
use rusqlite::Connection;
use serde_json::{json, Value};

const SENHA: &str = "Oficina2026x";

fn admin() -> Contexto {
    Contexto { usuario: "admin".into(), perfil: "administrador".into(), computador: "teste".into() }
}

/// Todo o conteúdo textual da tabela `alteracoes`, linha por linha, coluna por coluna.
fn conteudo_da_fila(conn: &Connection) -> String {
    let mut stmt = conn.prepare("SELECT * FROM alteracoes").unwrap();
    let n = stmt.column_count();
    let linhas = stmt
        .query_map([], |r| {
            let mut s = String::new();
            for i in 0..n {
                if let Ok(Some(t)) = r.get::<_, Option<String>>(i) {
                    s.push_str(&t);
                    s.push('\n');
                }
            }
            Ok(s)
        })
        .unwrap();
    linhas.map(|l| l.unwrap()).collect()
}

/// Mesmo caminho que `escrever_offline` faz: prepara, executa e enfileira como pendente.
fn enfileirar(conn: &Connection, nome: &str, args: &Value) -> AppResultUnit {
    let preparado = alteracoes::preparar_args(nome, args)?;
    comandos::executar(conn, nome, &preparado, &admin())?;
    alteracoes::registrar(conn, &Nova {
        computador: "PC-CLIENTE", usuario: "admin", comando: nome, args: &preparado,
        tabela: comandos::tabela_do_comando(nome), id_local: None, base_hash: None, status: "pendente",
    })?;
    Ok(())
}
type AppResultUnit = oficina_core::error::AppResult<()>;

#[test]
fn senha_em_texto_puro_nunca_aparece_na_fila() {
    let conn = db::open_in_memory().unwrap();
    let f = comandos::executar(&conn, "funcionarios_criar", &json!({"dados": {"nome": "Carlos"}}), &admin()).unwrap();
    let args = json!({"id": f["id"], "dados": {"login": "carlos", "senha": SENHA, "perfil": "funcionario"}});
    enfileirar(&conn, "funcionarios_dar_acesso", &args).unwrap();

    let texto = conteudo_da_fila(&conn);
    assert!(!texto.contains(SENHA), "senha em texto puro na fila: {texto}");
    assert!(texto.contains("$argon2"), "a fila deveria guardar o hash");
    // O acesso local foi concedido com essa senha.
    assert!(funcionarios::autenticar(&conn, "carlos", SENHA).unwrap().is_some());
}

#[test]
fn senha_fraca_nao_chega_a_ser_enfileirada() {
    let conn = db::open_in_memory().unwrap();
    let f = comandos::executar(&conn, "funcionarios_criar", &json!({"dados": {"nome": "Carlos"}}), &admin()).unwrap();
    let args = json!({"id": f["id"], "dados": {"login": "carlos", "senha": "123", "perfil": "funcionario"}});
    assert!(enfileirar(&conn, "funcionarios_dar_acesso", &args).is_err());
    assert!(alteracoes::listar(&conn, &["pendente"]).unwrap().is_empty());
}

#[test]
fn dar_acesso_aceita_hash_pronto_e_recusa_formato_invalido() {
    let conn = db::open_in_memory().unwrap();
    let f = comandos::executar(&conn, "funcionarios_criar", &json!({"dados": {"nome": "Ana"}}), &admin()).unwrap();
    let id = f["id"].as_i64().unwrap();
    let hash = oficina_core::seguranca::hash_senha(SENHA).unwrap();
    let args = json!({"id": id, "dados": {"login": "ana", "senha_hash": hash, "perfil": "funcionario"}});
    comandos::executar(&conn, "funcionarios_dar_acesso", &args, &admin()).unwrap();
    assert!(funcionarios::autenticar(&conn, "ana", SENHA).unwrap().is_some());

    let ruim = json!({"id": id, "dados": {"login": "ana", "senha_hash": "texto-qualquer", "perfil": "funcionario"}});
    assert!(comandos::executar(&conn, "funcionarios_dar_acesso", &ruim, &admin()).is_err());
}

#[test]
fn fila_antiga_com_senha_em_claro_e_migrada_para_hash() {
    let conn = db::open_in_memory().unwrap();
    let f = comandos::executar(&conn, "funcionarios_criar", &json!({"dados": {"nome": "Carlos"}}), &admin()).unwrap();
    let antigo = json!({"id": f["id"], "dados": {"login": "carlos", "senha": SENHA, "perfil": "funcionario"}});
    // Linha como a versão anterior gravava: senha em texto puro, também dentro do conflito.
    let uuid = alteracoes::registrar(&conn, &Nova {
        computador: "PC", usuario: "admin", comando: "funcionarios_dar_acesso", args: &antigo,
        tabela: Some("funcionarios"), id_local: None, base_hash: None, status: "pendente",
    }).unwrap();
    conn.execute(
        "UPDATE alteracoes SET status = 'conflito', conflito = ?2 WHERE uuid = ?1",
        rusqlite::params![uuid, json!({"servidor": null, "local": antigo, "comando": "funcionarios_dar_acesso"}).to_string()],
    )
    .unwrap();
    assert!(conteudo_da_fila(&conn).contains(SENHA));

    assert_eq!(alteracoes::migrar_senhas_em_claro(&conn).unwrap(), 1);
    let texto = conteudo_da_fila(&conn);
    assert!(!texto.contains(SENHA), "migração deixou senha em claro: {texto}");
    assert!(texto.contains("$argon2"));
    // A linha continua na fila com o mesmo status.
    assert_eq!(alteracoes::listar(&conn, &["conflito"]).unwrap().len(), 1);
}

/// Cópia local com um cliente "Ana", uma alteração em conflito, uma pendente e uma em erro.
fn cenario_com_fila() -> Connection {
    let local = db::open_in_memory().unwrap();
    comandos::executar(&local, "clientes_criar", &json!({"dados": {"nome": "Ana"}}), &admin()).unwrap();
    alteracoes::registrar(&local, &Nova {
        computador: "PC", usuario: "admin", comando: "clientes_atualizar",
        args: &json!({"id": 1, "dados": {"nome": "Ana Local"}}),
        tabela: Some("clientes"), id_local: Some(1), base_hash: Some("base-antiga"), status: "conflito",
    }).unwrap();
    alteracoes::registrar(&local, &Nova {
        computador: "PC", usuario: "admin", comando: "clientes_criar",
        args: &json!({"dados": {"nome": "Bia"}}),
        tabela: Some("clientes"), id_local: None, base_hash: None, status: "pendente",
    }).unwrap();
    alteracoes::registrar(&local, &Nova {
        computador: "PC", usuario: "admin", comando: "clientes_atualizar",
        args: &json!({"id": 99, "dados": {"nome": "Ninguém"}}),
        tabela: Some("clientes"), id_local: Some(99), base_hash: None, status: "erro",
    }).unwrap();
    local
}

fn nomes_clientes(conn: &Connection) -> Vec<String> {
    let v = comandos::executar(conn, "clientes_listar", &json!({}), &admin()).unwrap();
    v.as_array().unwrap().iter().map(|c| c["nome"].as_str().unwrap().to_string()).collect()
}

#[test]
fn copia_do_servidor_nao_apaga_alteracoes_da_fila() {
    let mut local = cenario_com_fila();
    comandos::executar(&local, "clientes_criar", &json!({"dados": {"nome": "Fantasma"}}), &admin()).unwrap();

    // Servidor: Ana (versão do servidor) e Carlos, que só existe lá.
    let servidor = db::open_in_memory().unwrap();
    comandos::executar(&servidor, "clientes_criar", &json!({"dados": {"nome": "Ana"}}), &admin()).unwrap();
    comandos::executar(&servidor, "clientes_criar", &json!({"dados": {"nome": "Carlos"}}), &admin()).unwrap();
    let snap = sync::snapshot(&servidor).unwrap();

    let refeitas = sync::aplicar_snapshot_preservando_fila(&mut local, &snap).unwrap();
    assert_eq!(refeitas, 2, "conflito e pendente refeitas; a de erro não cabe mais");

    let nomes = nomes_clientes(&local);
    assert!(nomes.contains(&"Ana Local".to_string()), "alteração em conflito sumiu: {nomes:?}");
    assert!(nomes.contains(&"Bia".to_string()), "alteração pendente sumiu: {nomes:?}");
    assert!(nomes.contains(&"Carlos".to_string()), "dado do servidor não chegou: {nomes:?}");
    assert!(!nomes.contains(&"Fantasma".to_string()), "registro sem fila deveria sair com a cópia nova");
    assert!(!nomes.contains(&"Ninguém".to_string()));

    // A fila continua intacta: nada foi marcado como sincronizado.
    let pendentes = alteracoes::listar(&local, &["pendente"]).unwrap();
    let conflitos = alteracoes::listar(&local, &["conflito"]).unwrap();
    let erros = alteracoes::listar(&local, &["erro"]).unwrap();
    assert_eq!((pendentes.len(), conflitos.len(), erros.len()), (1, 1, 1));
}

#[test]
fn copia_sem_fila_continua_igual_ao_servidor() {
    let mut local = db::open_in_memory().unwrap();
    let servidor = db::open_in_memory().unwrap();
    comandos::executar(&servidor, "clientes_criar", &json!({"dados": {"nome": "Ana"}}), &admin()).unwrap();
    let snap = sync::snapshot(&servidor).unwrap();
    assert_eq!(sync::aplicar_snapshot_preservando_fila(&mut local, &snap).unwrap(), 0);
    assert_eq!(sync::snapshot(&local).unwrap(), snap);
}
