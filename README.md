# Sistema de Oficina (Tauri + Rust)

Aplicativo desktop para gestão de oficina mecânica: clientes, veículos, equipe, peças/serviços com estoque, ordens de serviço (OS) com fluxo de status, caixa, registro de atividades, impressão de cupom e leitor de código de barras.

## Estrutura

```
sistema-oficina/
├── Cargo.toml                  # workspace Rust
├── crates/oficina-core/        # regras de negócio + SQLite (sem UI, testável)
│   ├── src/db.rs               # esquema e conexão
│   ├── src/validation.rs       # validação acumulativa (placa, dinheiro, faixas)
│   ├── src/clientes.rs         # um arquivo por cadastro
│   ├── src/veiculos.rs
│   ├── src/funcionarios.rs
│   ├── src/itens.rs            # peças e serviços (campo `tipo`)
│   ├── src/ordens/rules.rs     # máquina de estados da OS (puro)
│   ├── src/ordens/service.rs   # criar, itens, status, estoque, exclusão
│   ├── src/dashboard.rs       # indicadores do painel
│   ├── src/caixa.rs           # abertura, recebimentos por forma de pagamento e fechamento
│   ├── src/registro.rs        # registro de atividades (sem senhas)
│   ├── src/impressao.rs       # cupom ESC/POS (crate escpos) e envio à impressora
│   └── tests/                 # regras, caixa, acesso, sincronia, impressão
├── src-tauri/                  # camada Tauri: comandos que chamam o core
│   └── src/                    # comandos Tauri, rede, sincronia, servidor e backup
└── ui/                         # interface (HTML + JS modular, sem build)
    ├── data/veiculos-brasileiros.json  # catálogo de veículos (busca no cadastro)
    ├── js/pages/               # uma página por tela
    └── js/crud.js              # página CRUD genérica (cadastros simples)
```

## Atalhos de teclado

| Tecla | Tela |
|---|---|
| F1 | Ajuda |
| F2 | Nova Ordem de Serviço |
| F3 | Carros na Oficina |
| F4 | Peças e Serviços |
| F5 | Clientes e Veículos |
| F6 | Caixa |
| F7 | Funcionários (administrador) |
| F8 | Configurações (administrador) |
| F9 | Registro de atividades (administrador) |
| Esc | Volta para a tela inicial |

## Acesso e segurança

- Cada funcionário entra com login e senha (mínimo de 8 caracteres). As senhas ficam com hash Argon2.
- Depois de 3 senhas erradas seguidas, o acesso fica bloqueado por 5 minutos e a tela avisa o horário em que pode tentar de novo.
- Senhas não entram no registro de atividades nem na fila de sincronização depois de enviadas.
- Configurações, funcionários, registro de atividades e histórico de sincronização são só do administrador.

## Pré-requisitos

- Rust estável (`rustup`), já com `cargo` 1.7x ou superior.
- Bibliotecas de sistema do Tauri (Linux/Debian/Ubuntu):

```bash
sudo apt install build-essential pkg-config libssl-dev libdbus-1-dev \
  libwebkit2gtk-4.1-dev libjavascriptcoregtk-4.1-dev libsoup-3.0-dev \
  libgtk-3-dev librsvg2-dev libayatana-appindicator3-dev
```

## Comandos

```bash
cargo test -p oficina-core          # testes das regras (não precisam de GTK)
cargo install tauri-cli --version "^2"   # uma vez
cargo tauri dev                     # roda o aplicativo
cargo tauri build                   # gera o instalador
```

O banco fica no diretório de dados do aplicativo (`oficina.db`). Ele é copiado automaticamente a cada 6 horas.

Modo de rede: na primeira execução o computador escolhe **Só este computador** (sem rede), **Computador principal** ou **Usar o computador principal**. Em Configurações, o administrador pode trocar essa escolha. No modo *Só este computador* o aplicativo não abre nenhuma porta de rede.

## De onde veio cada parte

| Fonte | O que foi aproveitado | O que ficou de fora |
|---|---|---|
| `oficina.md` (BeepStart, base principal) | Cadastro de cliente com CPF e observações; peças com código, custo, venda e estoque; OS com fluxo `CRIADA → EM_ANDAMENTO → CONCLUÍDA`; painel com faturamento do mês e estoque baixo; caixa | PDV completo, financeiro (contas a pagar e receber), locação, NF-e, delivery, fidelidade, compras e fornecedores |
| `AllyPts/crud-oficina-mecanica` | Entidades cliente, veículo, funcionário, item e OS; OS ligada a veículo e responsável | Credenciais de banco que aparecem no README (não foram copiadas); backend Express/Sequelize; OS sem regra de estoque |
| `fvandrad/oficina-mecanica` | Separação entre peças e serviços; campos de dashboard | Persistência em JSON Server; sem regras de OS |

Decisões tomadas (podem ser alteradas):

- **Tauri + Rust**, com o núcleo em crate separado para testar sem interface.
- **SQLite embutido** (`rusqlite`, bundled): o app funciona sem servidor nem configuração.
- **Dinheiro em `f64` arredondado a centavos.** Para um PDV com muito volume, convém migrar para inteiros em centavos.
- **Login por funcionário**, com bloqueio após 3 erros (ver seção Acesso e segurança).
- **Peças e serviços na mesma tabela** (`tipo`), como no BeepStart; só peças consomem estoque.
- **Estoque baixado na conclusão da OS**, não na criação. Se faltar peça, a conclusão é recusada e nada muda (transação).
- **Concluir a OS é receber:** exige caixa aberto e forma de pagamento (dinheiro, PIX, cartão ou outra).
- **OS só edita itens enquanto está `CRIADA`**, e só exclui `CRIADA` ou `CANCELADA`.

## Catálogo de veículos

`ui/data/veiculos-brasileiros.json` (marca, modelo, ano, motor). O catálogo não é fonte confiável para motor: confira o motor antes de salvar. Na tela de Veículos, a busca filtra esse catálogo; se não achar, o botão **Cadastrar novo veículo** abre o formulário com o texto digitado. 202 registros não têm ano, e nesses casos o campo fica vazio para preencher.

## Testes

`cargo test -p oficina-core` cobre: regras da OS (`tests/ordens.rs`), validação (`tests/validacao.rs`), caixa (`tests/caixa.rs`), bloqueio de login (`tests/acesso.rs`), sincronização e conflitos, leitor de código e impressão em rede (`tests/leitor_impressao.rs`).

Testes de interface (WebDriver com tauri-driver) não fazem parte do `cargo test`.
