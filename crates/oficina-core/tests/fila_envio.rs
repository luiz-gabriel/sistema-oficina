// Fila offline: envio com identificador único (sem aplicar duas vezes), aceite do servidor antes de marcar
// e ordem de envio com dependências (cliente antes de veículo, veículo antes de OS).
use oficina_core::alteracoes::{self, Alteracao, Nova};
use oficina_core::comandos::{self, Contexto};
use oficina_core::db;
use oficina_core::error::{AppError, AppResult};
use oficina_core::sync::{self, Escrita, Resposta};
use rusqlite::Connection;
use serde_json::{json, Value};

fn admin() -> Contexto {
    Contexto { usuario: "admin".into(), perfil: "administrador".into(), computador: "PC-CLIENTE".into() }
}

/// Executa o comando na cópia local e enfileira como pendente (mesmo caminho do modo offline).
fn enfileirar(local: &Connection, nome: &str, args: &Value) -> String {
    let valor = comandos::executar(local, nome, args, &admin()).unwrap();
    let tabela = comandos::tabela_do_comando(nome);
    let id_local = if comandos::cria_registro(nome) {
        let id = comandos::id_criado(&valor);
        if let (Some(t), Some(i)) = (tabela, id) {
            alteracoes::registrar_id_local(local, t, i).unwrap();
        }
        id
    } else {
        comandos::alvo_id(nome, args)
    };
    alteracoes::registrar(local, &Nova {
        computador: "PC-CLIENTE", usuario: "admin", comando: nome, args, tabela, id_local, base_hash: None, status: "pendente",
    })
    .unwrap()
}

/// Servidor com `n` clientes já cadastrados, para que os ids do servidor sejam diferentes dos locais.
fn servidor_com_clientes(n: usize) -> Connection {
    let s = db::open_in_memory().unwrap();
    for i in 0..n {
        comandos::executar(&s, "clientes_criar", &json!({"dados": {"nome": format!("Cliente {i}")}}), &admin()).unwrap();
    }
    s
}

/// Envio que o cliente faz com um item: prepara, manda ao servidor e só marca como sincronizado com o aceite.
/// Devolve `Ok(false)` se o item precisa esperar uma dependência.
fn enviar(local: &Connection, servidor: &Connection, item: &Alteracao, perfil: &str) -> AppResult<bool> {
    let Some(args) = alteracoes::preparar_envio(local, item)? else { return Ok(false) };
    let ctx = Contexto { usuario: item.usuario.clone(), perfil: perfil.into(), computador: item.computador.clone() };
    let escrita = Escrita { uuid: Some(&item.uuid), nome: &item.comando, args: &args, base: None, ctx: &ctx };
    match sync::processar_escrita(servidor, &escrita)? {
        Resposta::Aplicada(valor) | Resposta::Repetida(valor) => {
            if comandos::cria_registro(&item.comando) {
                if let (Some(t), Some(idl), Some(ids)) = (item.tabela.as_deref(), item.id_local, valor["id"].as_i64()) {
                    alteracoes::definir_id_servidor(local, t, idl, ids)?;
                }
            }
            alteracoes::atualizar(local, &item.uuid, "sincronizado", None, None)?;
            Ok(true)
        }
        Resposta::Conflito(atual) => {
            let conflito = json!({"servidor": atual, "local": item.args, "comando": item.comando});
            alteracoes::atualizar(local, &item.uuid, "conflito", None, Some(&conflito))?;
            Ok(false)
        }
    }
}

/// O que acontece quando a resposta se perde: o servidor aplica, mas o cliente só vê uma falha de comunicação.
fn envio_com_resposta_perdida(local: &Connection, servidor: &Connection, item: &Alteracao) -> AppResult<()> {
    let args = alteracoes::preparar_envio(local, item)?.expect("sem dependências pendentes");
    let ctx = Contexto { usuario: item.usuario.clone(), perfil: "administrador".into(), computador: item.computador.clone() };
    let escrita = Escrita { uuid: Some(&item.uuid), nome: &item.comando, args: &args, base: None, ctx: &ctx };
    sync::processar_escrita(servidor, &escrita)?;
    Err(AppError::Offline("Sem comunicação com o servidor da oficina (conexão caiu).".into()))
}

fn contar_clientes(conn: &Connection, nome: &str) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM clientes WHERE nome = ?1", [nome], |r| r.get(0)).unwrap()
}

#[test]
fn resposta_perdida_nao_aplica_a_alteracao_duas_vezes() {
    let servidor = servidor_com_clientes(3);
    let local = db::open_in_memory().unwrap();
    enfileirar(&local, "clientes_criar", &json!({"dados": {"nome": "Ana"}}));
    let item = alteracoes::listar(&local, &["pendente"]).unwrap().remove(0);

    // Falha no meio do envio: o servidor aplicou, mas o cliente não recebeu a resposta.
    assert!(envio_com_resposta_perdida(&local, &servidor, &item).is_err());
    assert_eq!(contar_clientes(&servidor, "Ana"), 1);
    // Sem aceite, a alteração continua pendente e não foi marcada como sincronizada.
    assert_eq!(alteracoes::listar(&local, &["pendente"]).unwrap().len(), 1);

    // Novo envio da mesma alteração (mesmo identificador): o servidor reconhece e não aplica de novo.
    assert!(enviar(&local, &servidor, &item, "administrador").unwrap());
    assert_eq!(contar_clientes(&servidor, "Ana"), 1, "a alteração foi aplicada duas vezes");
    assert!(alteracoes::listar(&local, &["pendente"]).unwrap().is_empty());
    // O id que o servidor deu ao registro foi guardado, para as próximas alterações apontarem para ele.
    let id_servidor: i64 = servidor.query_row("SELECT id FROM clientes WHERE nome = 'Ana'", [], |r| r.get(0)).unwrap();
    assert_eq!(alteracoes::situacao_id(&local, "clientes", item.id_local.unwrap()).unwrap(), Some(Some(id_servidor)));
}

#[test]
fn recusa_do_servidor_mantem_a_alteracao_na_fila() {
    let servidor = servidor_com_clientes(1);
    let local = db::open_in_memory().unwrap();
    // Funcionário criado offline por um administrador; o envio sai com perfil sem permissão para cadastrar.
    enfileirar(&local, "funcionarios_criar", &json!({"dados": {"nome": "Carlos"}}));
    let item = alteracoes::listar(&local, &["pendente"]).unwrap().remove(0);

    assert!(enviar(&local, &servidor, &item, "funcionario").is_err());
    let pendentes = alteracoes::listar(&local, &["pendente"]).unwrap();
    assert_eq!(pendentes.len(), 1, "a alteração sumiu da fila");
    assert_eq!(pendentes[0].uuid, item.uuid);
}

#[test]
fn fila_envia_na_ordem_e_espera_a_dependencia() {
    let servidor = servidor_com_clientes(2);
    let local = db::open_in_memory().unwrap();
    enfileirar(&local, "clientes_criar", &json!({"dados": {"nome": "Ana"}}));
    let id_cliente_local = alteracoes::listar(&local, &["pendente"]).unwrap()[0].id_local.unwrap();
    enfileirar(&local, "veiculos_criar", &json!({"dados": {"cliente_id": id_cliente_local, "placa": "ABC1D23", "modelo": "Gol", "motor": "1.6"}}));
    let id_veiculo_local = alteracoes::listar(&local, &["pendente"]).unwrap()[1].id_local.unwrap();
    // OS enfileirada depois do veículo (criada à mão: a execução exige funcionário e itens cadastrados).
    alteracoes::registrar(&local, &Nova {
        computador: "PC-CLIENTE", usuario: "admin", comando: "ordens_criar",
        args: &json!({"dados": {"veiculo_id": id_veiculo_local, "funcionario_id": 1, "itens": []}}),
        tabela: Some("ordens_servico"), id_local: None, base_hash: None, status: "pendente",
    })
    .unwrap();

    // Ordem: a fila devolve na sequência em que as alterações aconteceram.
    let ordem: Vec<String> = alteracoes::listar(&local, &["pendente"]).unwrap().into_iter().map(|a| a.comando).collect();
    assert_eq!(ordem, ["clientes_criar", "veiculos_criar", "ordens_criar"]);

    let itens = alteracoes::listar(&local, &["pendente"]).unwrap();
    // Dependências: veículo e OS esperam o cliente (ainda não enviado).
    assert!(!enviar(&local, &servidor, &itens[1], "administrador").unwrap());
    assert!(!enviar(&local, &servidor, &itens[2], "administrador").unwrap());
    assert_eq!(alteracoes::listar(&local, &["pendente"]).unwrap().len(), 3, "esperar não pode mudar o estado");

    // Cliente primeiro; depois o veículo, que já sai apontando para o cliente do servidor.
    assert!(enviar(&local, &servidor, &itens[0], "administrador").unwrap());
    assert!(enviar(&local, &servidor, &itens[1], "administrador").unwrap());
    let id_cliente_servidor: i64 = servidor.query_row("SELECT id FROM clientes WHERE nome = 'Ana'", [], |r| r.get(0)).unwrap();
    let cliente_do_veiculo: i64 = servidor
        .query_row("SELECT cliente_id FROM veiculos WHERE placa = 'ABC1D23'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(cliente_do_veiculo, id_cliente_servidor);

    // A OS agora sai apontando para o veículo do servidor.
    let id_veiculo_servidor: i64 = servidor.query_row("SELECT id FROM veiculos WHERE placa = 'ABC1D23'", [], |r| r.get(0)).unwrap();
    let preparada = alteracoes::preparar_envio(&local, &itens[2]).unwrap().expect("veículo já enviado");
    assert_eq!(preparada["dados"]["veiculo_id"], json!(id_veiculo_servidor));
}
