// A exportação manual não pode levar senhas, hashes de senha nem chaves/tokens da configuração.
use oficina_core::db;
use oficina_core::sync;

#[test]
fn exportacao_nao_contem_hash_de_senha_nem_chaves() {
    let conn = db::open_in_memory().unwrap();
    conn.execute_batch(
        "INSERT INTO funcionarios (id, nome, login, senha_hash, perfil) VALUES
           (1, 'Carlos', 'carlos', '$argon2id$v=19$m=19456,t=2,p=1$SEGREDOSALDOHASH$SEGREDOHASHCONTEUDO', 'administrador');
         INSERT INTO clientes (id, nome) VALUES (1, 'Ana');
         INSERT INTO config (chave, valor) VALUES
           ('chave_rede', 'CHAVESECRETAREDE0123456789'),
           ('nuvem_token', 'TOKENSECRETO987'),
           ('nuvem_client_segredo', 'SEGREDOCLIENTE555');",
    )
    .unwrap();

    // Sanidade: o snapshot de sincronização (que leva as contas aos outros computadores) ainda tem o hash.
    let sincronia = sync::snapshot(&conn).unwrap().to_string();
    assert!(sincronia.contains("SEGREDOHASHCONTEUDO"));

    let exportado = sync::snapshot_exportacao(&conn).unwrap().to_string();
    assert!(exportado.contains("Carlos"), "os dados do funcionário continuam no arquivo");
    assert!(!exportado.contains("senha_hash"));
    assert!(!exportado.contains("SEGREDOHASHCONTEUDO"));
    assert!(!exportado.contains("argon2"));
    assert!(!exportado.contains("CHAVESECRETAREDE"));
    assert!(!exportado.contains("TOKENSECRETO"));
    assert!(!exportado.contains("SEGREDOCLIENTE"));
}
