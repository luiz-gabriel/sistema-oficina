use rusqlite::Connection;
use std::path::Path;

use crate::error::AppResult;

/// Esquema completo. Usa IF NOT EXISTS, então pode rodar a cada inicialização.
pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS clientes (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  nome        TEXT NOT NULL,
  telefone    TEXT,
  email       TEXT,
  cpf         TEXT,
  endereco    TEXT,
  observacoes TEXT
);
CREATE TABLE IF NOT EXISTS veiculos (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  cliente_id INTEGER NOT NULL REFERENCES clientes(id),
  placa      TEXT NOT NULL UNIQUE,
  marca      TEXT,
  modelo     TEXT NOT NULL,
  ano        INTEGER,
  motor      TEXT,
  cor        TEXT,
  responsavel_id INTEGER REFERENCES funcionarios(id)
);
CREATE TABLE IF NOT EXISTS funcionarios (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  nome       TEXT NOT NULL,
  cargo      TEXT,
  telefone   TEXT,
  login      TEXT,
  senha_hash TEXT,
  perfil     TEXT NOT NULL DEFAULT 'funcionario' CHECK (perfil IN ('administrador', 'funcionario'))
);
CREATE TABLE IF NOT EXISTS fotos_veiculos (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  veiculo_id INTEGER NOT NULL REFERENCES veiculos(id) ON DELETE CASCADE,
  nome       TEXT NOT NULL,
  mime       TEXT NOT NULL,
  dados      BLOB NOT NULL,
  criado_em  TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS config (
  chave TEXT PRIMARY KEY,
  valor TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS alteracoes (
  seq          INTEGER PRIMARY KEY AUTOINCREMENT,
  uuid         TEXT NOT NULL UNIQUE,
  criado_em    TEXT NOT NULL,
  computador   TEXT NOT NULL,
  usuario      TEXT NOT NULL,
  comando      TEXT NOT NULL,
  args         TEXT NOT NULL,
  tabela       TEXT,
  id_local     INTEGER,
  base_hash    TEXT,
  forcar       INTEGER NOT NULL DEFAULT 0,
  status       TEXT NOT NULL CHECK (status IN ('local', 'pendente', 'sincronizado', 'conflito', 'erro', 'descartado')),
  erro         TEXT,
  conflito     TEXT
);
CREATE TABLE IF NOT EXISTS ids_locais (
  tabela      TEXT NOT NULL,
  id_local    INTEGER NOT NULL,
  id_servidor INTEGER,
  PRIMARY KEY (tabela, id_local)
);
CREATE TABLE IF NOT EXISTS itens (
  id             INTEGER PRIMARY KEY AUTOINCREMENT,
  nome           TEXT NOT NULL,
  tipo           TEXT NOT NULL CHECK (tipo IN ('peca', 'servico')),
  codigo         TEXT,
  preco_custo    REAL NOT NULL DEFAULT 0,
  preco_venda    REAL NOT NULL,
  quantidade     INTEGER NOT NULL DEFAULT 0,
  estoque_minimo INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS ordens_servico (
  id             INTEGER PRIMARY KEY AUTOINCREMENT,
  tipo           TEXT NOT NULL DEFAULT 'OS' CHECK (tipo IN ('OS', 'BALCAO')),
  veiculo_id     INTEGER REFERENCES veiculos(id),
  cliente_id     INTEGER REFERENCES clientes(id),
  funcionario_id INTEGER NOT NULL REFERENCES funcionarios(id),
  descricao      TEXT,
  status         TEXT NOT NULL CHECK (status IN ('CRIADA', 'EM_ANDAMENTO', 'CONCLUIDA', 'CANCELADA')),
  data_entrada   TEXT NOT NULL,
  data_conclusao TEXT,
  valor_total    REAL NOT NULL DEFAULT 0,
  pagamento      TEXT,
  orcamento_id   INTEGER,
  CHECK ((tipo = 'OS' AND veiculo_id IS NOT NULL) OR (tipo = 'BALCAO' AND veiculo_id IS NULL))
);
CREATE TABLE IF NOT EXISTS fidelidade_movimentos (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  cliente_id INTEGER NOT NULL REFERENCES clientes(id),
  pontos     INTEGER NOT NULL CHECK (pontos <> 0),
  origem     TEXT NOT NULL CHECK (origem IN ('OS', 'VENDA', 'RESGATE')),
  referencia INTEGER,
  criado_em  TEXT NOT NULL,
  UNIQUE (origem, referencia)
);
CREATE TABLE IF NOT EXISTS tentativas_login (
  login         TEXT PRIMARY KEY,
  falhas        INTEGER NOT NULL DEFAULT 0,
  bloqueado_ate TEXT
);
CREATE TABLE IF NOT EXISTS registro (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  criado_em  TEXT NOT NULL,
  usuario    TEXT NOT NULL,
  computador TEXT NOT NULL,
  acao       TEXT NOT NULL,
  detalhe    TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS caixas (
  id               INTEGER PRIMARY KEY AUTOINCREMENT,
  aberto_em        TEXT NOT NULL,
  aberto_por       TEXT NOT NULL,
  valor_abertura   REAL NOT NULL DEFAULT 0,
  fechado_em       TEXT,
  fechado_por      TEXT,
  valor_contado    REAL,
  dinheiro_esperado REAL,
  diferenca        REAL,
  observacao       TEXT
);
-- Itens de OS e de venda de balcão. `custo_unitario_centavos` é o custo efetivo gravado na saída da peça
-- (baixa de estoque). Serviços e linhas antigas ficam NULL.
CREATE TABLE IF NOT EXISTS ordem_itens (
  id                      INTEGER PRIMARY KEY AUTOINCREMENT,
  os_id                   INTEGER NOT NULL REFERENCES ordens_servico(id) ON DELETE CASCADE,
  item_id                 INTEGER NOT NULL REFERENCES itens(id),
  quantidade              INTEGER NOT NULL CHECK (quantidade > 0),
  preco_unitario          REAL NOT NULL,
  subtotal                REAL NOT NULL,
  custo_unitario_centavos INTEGER CHECK (custo_unitario_centavos IS NULL OR custo_unitario_centavos >= 0)
);
CREATE TABLE IF NOT EXISTS caixa_movimentos (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  caixa_id   INTEGER NOT NULL REFERENCES caixas(id),
  tipo       TEXT NOT NULL CHECK (tipo IN ('sangria', 'suprimento')),
  valor      REAL NOT NULL CHECK (valor > 0),
  motivo     TEXT NOT NULL CHECK (length(trim(motivo)) > 0),
  usuario    TEXT NOT NULL,
  criado_em  TEXT NOT NULL
);
-- Dados do atendimento do veículo (quilometragem, prazo, garantia, revisão), uma linha por OS.
CREATE TABLE IF NOT EXISTS ordem_km (
  os_id                INTEGER PRIMARY KEY REFERENCES ordens_servico(id) ON DELETE CASCADE,
  km_entrada           INTEGER,
  km_entrega           INTEGER,
  prazo_previsto       TEXT,
  garantia_dias        INTEGER,
  revisao              INTEGER NOT NULL DEFAULT 0 CHECK (revisao IN (0, 1)),
  observacoes_tecnicas TEXT
);
-- Fornecedores: documento é CPF (11 dígitos) ou CNPJ (14 dígitos), guardado só com números.
CREATE TABLE IF NOT EXISTS fornecedores (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  nome        TEXT NOT NULL,
  documento   TEXT NOT NULL UNIQUE,
  telefone    TEXT,
  email       TEXT,
  endereco    TEXT,
  observacoes TEXT
);
-- Compra: valores em centavos. `pagamento` é a forma escolhida ou 'a pagar'; `paga` = 1 quando já foi paga.
CREATE TABLE IF NOT EXISTS compras (
  id             INTEGER PRIMARY KEY AUTOINCREMENT,
  fornecedor_id  INTEGER NOT NULL REFERENCES fornecedores(id),
  data           TEXT NOT NULL,
  total_centavos INTEGER NOT NULL CHECK (total_centavos >= 0),
  pagamento      TEXT NOT NULL,
  paga           INTEGER NOT NULL CHECK (paga IN (0, 1)),
  criado_em      TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS compra_itens (
  id                      INTEGER PRIMARY KEY AUTOINCREMENT,
  compra_id               INTEGER NOT NULL REFERENCES compras(id) ON DELETE CASCADE,
  item_id                 INTEGER NOT NULL REFERENCES itens(id),
  quantidade              INTEGER NOT NULL CHECK (quantidade > 0),
  custo_unitario_centavos INTEGER NOT NULL CHECK (custo_unitario_centavos >= 0),
  subtotal_centavos       INTEGER NOT NULL CHECK (subtotal_centavos >= 0)
);
-- LEGADO: era a conta das compras a prazo. Agora a conta fica em contas_pagar (compra_id); esta tabela só é lida
-- pela migração (migrar_contas_pagar_compra) e não é mais gravada. Não apagar: guarda o histórico de bancos antigos.
CREATE TABLE IF NOT EXISTS contas_pagar_compra (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  compra_id       INTEGER NOT NULL UNIQUE REFERENCES compras(id) ON DELETE CASCADE,
  valor_centavos  INTEGER NOT NULL CHECK (valor_centavos >= 0),
  vencimento      TEXT NOT NULL,
  pago            INTEGER NOT NULL DEFAULT 0 CHECK (pago IN (0, 1))
);
CREATE TABLE IF NOT EXISTS orcamentos (
  id                   INTEGER PRIMARY KEY AUTOINCREMENT,
  tipo                 TEXT NOT NULL DEFAULT 'ORCAMENTO' CHECK (tipo IN ('ORCAMENTO', 'ADITIVO')),
  cliente_id           INTEGER NOT NULL REFERENCES clientes(id),
  veiculo_id           INTEGER NOT NULL REFERENCES veiculos(id),
  funcionario_id       INTEGER NOT NULL REFERENCES funcionarios(id),
  ordem_id             INTEGER REFERENCES ordens_servico(id),
  status               TEXT NOT NULL CHECK (status IN ('RASCUNHO', 'AGUARDANDO_APROVACAO', 'APROVADO', 'RECUSADO', 'CANCELADO')),
  observacoes          TEXT,
  validade             TEXT,
  prazo_dias           INTEGER CHECK (prazo_dias IS NULL OR prazo_dias >= 0),
  criado_em            TEXT NOT NULL,
  enviado_em          TEXT,
  aprovado_em          TEXT,
  aprovacao_forma      TEXT CHECK (aprovacao_forma IS NULL OR aprovacao_forma IN ('presencial', 'telefone', 'WhatsApp', 'outro')),
  aprovacao_obs        TEXT,
  recusado_em          TEXT,
  cancelado_em         TEXT,
  valor_total_centavos INTEGER NOT NULL DEFAULT 0 CHECK (valor_total_centavos >= 0),
  CHECK (tipo = 'ORCAMENTO' OR ordem_id IS NOT NULL)
);
CREATE TABLE IF NOT EXISTS orcamento_itens (
  id                      INTEGER PRIMARY KEY AUTOINCREMENT,
  orcamento_id            INTEGER NOT NULL REFERENCES orcamentos(id) ON DELETE CASCADE,
  item_id                 INTEGER NOT NULL REFERENCES itens(id),
  nome                    TEXT NOT NULL,
  tipo                    TEXT NOT NULL CHECK (tipo IN ('peca', 'servico')),
  quantidade              INTEGER NOT NULL CHECK (quantidade > 0),
  preco_unitario_centavos INTEGER NOT NULL CHECK (preco_unitario_centavos >= 0),
  subtotal_centavos       INTEGER NOT NULL CHECK (subtotal_centavos >= 0)
);
-- Recebimentos: cada entrada de dinheiro de uma ordem (sinal, parcial ou parte de pagamento dividido).
-- Venda de balcão também é ordem (tipo BALCAO), então usa ordem_id. Valores em centavos.
CREATE TABLE IF NOT EXISTS recebimentos (
  id             INTEGER PRIMARY KEY AUTOINCREMENT,
  ordem_id       INTEGER NOT NULL REFERENCES ordens_servico(id),
  valor_centavos INTEGER NOT NULL CHECK (valor_centavos > 0),
  forma          TEXT NOT NULL CHECK (forma IN ('dinheiro', 'pix', 'cartao', 'outro')),
  data           TEXT NOT NULL,
  usuario        TEXT NOT NULL,
  caixa_id       INTEGER REFERENCES caixas(id)
);
CREATE INDEX IF NOT EXISTS idx_recebimentos_ordem ON recebimentos(ordem_id);
-- Contas a receber: uma por dívida (OS ou venda). A situação (ABERTA, PARCIAL, QUITADA, VENCIDA) é calculada.
CREATE TABLE IF NOT EXISTS contas_receber (
  id                   INTEGER PRIMARY KEY AUTOINCREMENT,
  origem               TEXT NOT NULL CHECK (origem IN ('OS', 'VENDA', 'MANUAL')),
  referencia           INTEGER,
  cliente_id           INTEGER REFERENCES clientes(id),
  descricao            TEXT,
  valor_total_centavos INTEGER NOT NULL CHECK (valor_total_centavos >= 0),
  saldo_centavos       INTEGER NOT NULL CHECK (saldo_centavos >= 0),
  vencimento           TEXT NOT NULL,
  data                 TEXT NOT NULL,
  UNIQUE (origem, referencia)
);
-- Contas a pagar (despesas avulsas). `compra_id` é opcional. Situação (ABERTA, PARCIAL, PAGA, VENCIDA) é calculada.
CREATE TABLE IF NOT EXISTS contas_pagar (
  id             INTEGER PRIMARY KEY AUTOINCREMENT,
  descricao      TEXT NOT NULL CHECK (length(trim(descricao)) > 0),
  fornecedor     TEXT,
  compra_id      INTEGER REFERENCES compras(id),
  valor_centavos INTEGER NOT NULL CHECK (valor_centavos > 0),
  vencimento     TEXT NOT NULL,
  data           TEXT NOT NULL,
  usuario        TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS pagamentos_pagar (
  id             INTEGER PRIMARY KEY AUTOINCREMENT,
  conta_id       INTEGER NOT NULL REFERENCES contas_pagar(id),
  valor_centavos INTEGER NOT NULL CHECK (valor_centavos > 0),
  data           TEXT NOT NULL,
  usuario        TEXT NOT NULL
);
"#;

/// Abre (ou cria) o banco de arquivo e aplica o esquema.
pub fn open(path: &Path) -> AppResult<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| crate::error::AppError::Database(e.to_string()))?;
    }
    init(Connection::open(path)?)
}

/// Banco em memória, usado nos testes.
pub fn open_in_memory() -> AppResult<Connection> {
    init(Connection::open_in_memory()?)
}

/// Aplica o esquema e as migrações a uma conexão já aberta (usado pelos testes de migração de bancos antigos).
pub fn init(conn: Connection) -> AppResult<Connection> {
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    conn.execute_batch(SCHEMA)?;
    migrar(&conn)?;
    // Índice só depois da migração: bancos antigos ainda não têm a coluna login.
    conn.execute_batch("CREATE UNIQUE INDEX IF NOT EXISTS idx_funcionarios_login ON funcionarios(login);")?;
    Ok(conn)
}

/// Migrações para bancos criados por versões anteriores (CREATE TABLE IF NOT EXISTS não altera tabelas existentes).
fn migrar(conn: &Connection) -> AppResult<()> {
    let colunas: Vec<String> = conn
        .prepare("PRAGMA table_info(veiculos)")?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<Result<_, _>>()?;
    if !colunas.iter().any(|c| c == "motor") {
        conn.execute_batch("ALTER TABLE veiculos ADD COLUMN motor TEXT;")?;
    }
    if !colunas.iter().any(|c| c == "cor") {
        conn.execute_batch("ALTER TABLE veiculos ADD COLUMN cor TEXT;")?;
    }
    if !colunas.iter().any(|c| c == "responsavel_id") {
        conn.execute_batch("ALTER TABLE veiculos ADD COLUMN responsavel_id INTEGER REFERENCES funcionarios(id);")?;
    }
    let colunas_func: Vec<String> = conn
        .prepare("PRAGMA table_info(funcionarios)")?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<Result<_, _>>()?;
    for (coluna, definicao) in [
        ("login", "TEXT"),
        ("senha_hash", "TEXT"),
        ("perfil", "TEXT NOT NULL DEFAULT 'funcionario' CHECK (perfil IN ('administrador', 'funcionario'))"),
    ] {
        if !colunas_func.iter().any(|c| c == coluna) {
            conn.execute_batch(&format!("ALTER TABLE funcionarios ADD COLUMN {coluna} {definicao};"))?;
        }
    }
    let colunas_os: Vec<String> = conn
        .prepare("PRAGMA table_info(ordens_servico)")?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<Result<_, _>>()?;
    if !colunas_os.iter().any(|c| c == "pagamento") {
        conn.execute_batch("ALTER TABLE ordens_servico ADD COLUMN pagamento TEXT;")?;
    }
    // Venda de balcão: veículo passou a ser opcional e a OS ganhou `tipo` e `cliente_id`.
    // SQLite não tira NOT NULL com ALTER, então a tabela é reconstruída.
    if !colunas_os.iter().any(|c| c == "tipo") {
        reconstruir_ordens_servico(conn)?;
    }
    // Estado AGUARDANDO_PAGAMENTO: o CHECK de status de bancos antigos não aceita o novo valor.
    let sql_os: String = conn.query_row(
        "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'ordens_servico'",
        [],
        |r| r.get(0),
    )?;
    if !sql_os.contains("AGUARDANDO_PAGAMENTO") {
        ampliar_status_os(conn, &sql_os)?;
    }
    // Custo efetivo da peça na saída. Linhas antigas ficam NULL e o relatório de lucro usa a média atual como fallback.
    let colunas_itens: Vec<String> = conn
        .prepare("PRAGMA table_info(ordem_itens)")?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<Result<_, _>>()?;
    if !colunas_itens.iter().any(|c| c == "custo_unitario_centavos") {
        conn.execute_batch(
            "ALTER TABLE ordem_itens ADD COLUMN custo_unitario_centavos INTEGER \
             CHECK (custo_unitario_centavos IS NULL OR custo_unitario_centavos >= 0);",
        )?;
    }
    migrar_contas_pagar_compra(conn)?;
    Ok(())
}

/// Compra a prazo: a conta passa a ser só `contas_pagar` (com `compra_id`). Converte as linhas antigas de
/// `contas_pagar_compra` que ainda não têm conta com o mesmo `compra_id`. Se a antiga estava paga (pago = 1),
/// lança um pagamento com o valor total e a data da compra. Idempotente. A tabela antiga não é apagada.
fn migrar_contas_pagar_compra(conn: &Connection) -> AppResult<()> {
    conn.execute_batch(
        "INSERT INTO contas_pagar (descricao, fornecedor, compra_id, valor_centavos, vencimento, data, usuario)
         SELECT 'Compra #' || cpc.compra_id, f.nome, cpc.compra_id, cpc.valor_centavos, cpc.vencimento, c.data, 'migração'
         FROM contas_pagar_compra cpc
         JOIN compras c ON c.id = cpc.compra_id
         JOIN fornecedores f ON f.id = c.fornecedor_id
         WHERE cpc.valor_centavos > 0
           AND NOT EXISTS (SELECT 1 FROM contas_pagar x WHERE x.compra_id = cpc.compra_id);
         INSERT INTO pagamentos_pagar (conta_id, valor_centavos, data, usuario)
         SELECT cp.id, cp.valor_centavos, c.data, 'migração'
         FROM contas_pagar_compra cpc
         JOIN contas_pagar cp ON cp.compra_id = cpc.compra_id AND cp.usuario = 'migração'
         JOIN compras c ON c.id = cpc.compra_id
         WHERE cpc.pago = 1
           AND NOT EXISTS (SELECT 1 FROM pagamentos_pagar p WHERE p.conta_id = cp.id);",
    )?;
    Ok(())
}

/// Recria `ordens_servico` aceitando o status AGUARDANDO_PAGAMENTO. Copia as linhas e mantém o contador de ids.
fn ampliar_status_os(conn: &Connection, sql_atual: &str) -> AppResult<()> {
    // O SQLite grava o nome com aspas depois de um RENAME, então troca-se o cabeçalho inteiro até o primeiro '('.
    let corpo = sql_atual
        .find('(')
        .map(|i| &sql_atual[i..])
        .ok_or_else(|| crate::error::AppError::Database("definição de ordens_servico inesperada".into()))?;
    let sql_nova = format!("CREATE TABLE ordens_servico_nova {corpo}")
        .replace("'EM_ANDAMENTO', 'CONCLUIDA'", "'EM_ANDAMENTO', 'AGUARDANDO_PAGAMENTO', 'CONCLUIDA'");
    if !sql_nova.contains("AGUARDANDO_PAGAMENTO") {
        return Err(crate::error::AppError::Database("não foi possível ampliar o status das ordens".into()));
    }
    conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
    let resultado = conn.execute_batch(&format!(
        "BEGIN;
         {sql_nova};
         INSERT INTO ordens_servico_nova SELECT * FROM ordens_servico;
         DELETE FROM sqlite_sequence WHERE name = 'ordens_servico_nova';
         INSERT INTO sqlite_sequence (name, seq)
           SELECT 'ordens_servico_nova', MAX(COALESCE((SELECT seq FROM sqlite_sequence WHERE name = 'ordens_servico'), 0),
                                              COALESCE((SELECT MAX(id) FROM ordens_servico), 0));
         DROP TABLE ordens_servico;
         ALTER TABLE ordens_servico_nova RENAME TO ordens_servico;
         COMMIT;"
    ));
    if resultado.is_err() {
        let _ = conn.execute_batch("ROLLBACK;");
    }
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    resultado.map_err(Into::into)
}

/// Recria `ordens_servico` no formato atual, copiando as linhas existentes (tipo 'OS').
/// Com as chaves estrangeiras desligadas, o DROP não apaga `ordem_itens` em cascata.
/// Preserva o contador de ids (sqlite_sequence) para não reaproveitar ids antigos.
fn reconstruir_ordens_servico(conn: &Connection) -> AppResult<()> {
    conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
    let resultado = conn.execute_batch(
        "BEGIN;
         CREATE TABLE ordens_servico_nova (
           id             INTEGER PRIMARY KEY AUTOINCREMENT,
           tipo           TEXT NOT NULL DEFAULT 'OS' CHECK (tipo IN ('OS', 'BALCAO')),
           veiculo_id     INTEGER REFERENCES veiculos(id),
           cliente_id     INTEGER REFERENCES clientes(id),
           funcionario_id INTEGER NOT NULL REFERENCES funcionarios(id),
           descricao      TEXT,
           status         TEXT NOT NULL CHECK (status IN ('CRIADA', 'EM_ANDAMENTO', 'CONCLUIDA', 'CANCELADA')),
           data_entrada   TEXT NOT NULL,
           data_conclusao TEXT,
           valor_total    REAL NOT NULL DEFAULT 0,
           pagamento      TEXT,
           CHECK ((tipo = 'OS' AND veiculo_id IS NOT NULL) OR (tipo = 'BALCAO' AND veiculo_id IS NULL))
         );
         INSERT INTO ordens_servico_nova (id, tipo, veiculo_id, cliente_id, funcionario_id, descricao, status,
                                          data_entrada, data_conclusao, valor_total, pagamento)
           SELECT id, 'OS', veiculo_id, NULL, funcionario_id, descricao, status,
                  data_entrada, data_conclusao, valor_total, pagamento FROM ordens_servico;
         INSERT INTO sqlite_sequence (name, seq)
           SELECT 'ordens_servico_nova', 0
           WHERE NOT EXISTS (SELECT 1 FROM sqlite_sequence WHERE name = 'ordens_servico_nova');
         UPDATE sqlite_sequence SET seq = MAX(seq, COALESCE((SELECT seq FROM sqlite_sequence WHERE name = 'ordens_servico'), 0))
           WHERE name = 'ordens_servico_nova';
         DROP TABLE ordens_servico;
         ALTER TABLE ordens_servico_nova RENAME TO ordens_servico;
         COMMIT;",
    );
    if resultado.is_err() {
        let _ = conn.execute_batch("ROLLBACK;");
    }
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    resultado.map_err(Into::into)
}
