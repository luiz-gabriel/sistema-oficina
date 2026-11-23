//! Despacho único dos comandos de negócio. A tela, o servidor da rede e a fila offline usam esta mesma função.
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use crate::error::{AppError, AppResult};
use crate::sync::{hash_linha, linha_json};
use crate::{clientes, dashboard, fotos, funcionarios, itens, orcamentos, ordens, veiculos};
use rusqlite::{Connection, OptionalExtension};

/// Quem está fazendo a operação.
#[derive(Debug, Clone)]
pub struct Contexto {
    pub usuario: String,
    pub perfil: String,
    pub computador: String,
}

const ESCRITA: &[&str] = &[
    "os_concluir_servico", "os_registrar_recebimento", "contas_pagar_criar", "contas_pagar_pagar",
    "clientes_criar", "clientes_atualizar", "clientes_remover",
    "veiculos_criar", "veiculos_atualizar", "veiculos_remover",
    "veiculos_foto_adicionar", "veiculos_foto_remover",
    "funcionarios_criar", "funcionarios_atualizar", "funcionarios_remover",
    "funcionarios_dar_acesso", "funcionarios_revogar_acesso",
    "itens_criar", "itens_atualizar", "itens_remover",
    "ordens_criar", "ordens_substituir_itens", "ordens_alterar_status", "ordens_excluir",
    "venda_balcao_criar",
    "orcamento_criar", "orcamento_enviar", "orcamento_aprovar", "orcamento_recusar", "orcamento_cancelar",
    "orcamento_converter_os", "orcamento_adicional",
    "ordens_dados_veiculo_salvar",
    "caixa_abrir", "caixa_fechar", "caixa_sangria", "caixa_suprimento",
];

pub const ADMINISTRADOR: &str = "administrador";

/// Regra de perfil para os comandos de negócio. Administrador pode tudo.
/// Funcionário pode usar as telas, mas não concede nem mexe no acesso ou no cadastro de um administrador.
/// `perfil_alvo` é o perfil atual do funcionário que o comando atinge (None se não existe ou não se aplica).
/// Perfil desconhecido recebe as restrições de funcionário.
pub fn pode_executar(perfil: &str, cmd: &str, _args: &Value, _perfil_alvo: Option<&str>) -> bool {
    if perfil == ADMINISTRADOR {
        return true;
    }
    match cmd {
        // Cadastro e acesso de funcionários: só o administrador (a leitura da lista continua livre).
        "funcionarios_criar" | "funcionarios_atualizar" | "funcionarios_remover"
        | "funcionarios_dar_acesso" | "funcionarios_revogar_acesso" => false,
        // Resultados financeiros e relatórios: só o administrador (funcionário vê apenas o próprio caixa aberto).
        "caixa_historico" | "financeiro_resumo" | "recebimentos_listar" | "contas_receber_listar"
        | "contas_pagar_listar" | "contas_pagar_criar" | "contas_pagar_pagar" | "painel_serie" => false,
        c if c.starts_with("relatorio_") => false,
        // Aparelhos e pareamento: só o administrador (a leitura também é restrita).
        c if c.starts_with("dispositivos_") || c.starts_with("pareamento_") => false,
        _ => true,
    }
}

/// Configurações da oficina (logo, impressora, modo de rede, backup, histórico de sincronização): só o administrador.
pub fn pode_configurar(perfil: &str) -> bool {
    perfil == ADMINISTRADOR
}

/// Registro de atividades: só o administrador consulta.
pub fn pode_ver_registro(perfil: &str) -> bool {
    perfil == ADMINISTRADOR
}

pub fn e_escrita(cmd: &str) -> bool {
    ESCRITA.contains(&cmd) || ESCRITA_COMPRAS.contains(&cmd)
}

/// Perfil atual do funcionário atingido por um comando de acesso (para a regra de administrador).
fn perfil_do_alvo(conn: &Connection, cmd: &str, args: &Value) -> AppResult<Option<String>> {
    if !matches!(cmd, "funcionarios_dar_acesso" | "funcionarios_revogar_acesso" | "funcionarios_remover") {
        return Ok(None);
    }
    let Some(id) = args.get("id").and_then(Value::as_i64) else { return Ok(None) };
    Ok(conn
        .query_row("SELECT perfil FROM funcionarios WHERE id = ?1", [id], |r| r.get::<_, String>(0))
        .optional()?)
}

/// Tabela afetada por um comando (None para leituras de resumo).
pub fn tabela_do_comando(cmd: &str) -> Option<&'static str> {
    if cmd.starts_with("fornecedores_") { Some("fornecedores") }
    else if cmd == "compras_registrar" { Some("compras") }
    else if cmd.starts_with("clientes_") { Some("clientes") }
    else if cmd.starts_with("veiculos_foto") { Some("fotos_veiculos") }
    else if cmd.starts_with("veiculos_") { Some("veiculos") }
    else if cmd.starts_with("funcionarios_") { Some("funcionarios") }
    else if cmd.starts_with("itens_") { Some("itens") }
    else if cmd.starts_with("ordens_") || cmd.starts_with("venda_balcao") { Some("ordens_servico") }
    else if cmd == "caixa_sangria" || cmd == "caixa_suprimento" { Some("caixa_movimentos") }
    else if cmd.starts_with("orcamento_") { Some("orcamentos") }
    else if cmd.starts_with("caixa_") { Some("caixas") }
    else { None }
}

/// Comandos que criam um registro novo (o id criado volta no resultado).
pub fn cria_registro(cmd: &str) -> bool {
    matches!(cmd, "clientes_criar" | "veiculos_criar" | "funcionarios_criar" | "itens_criar" | "ordens_criar" | "venda_balcao_criar" | "veiculos_foto_adicionar" | "caixa_abrir" | "caixa_sangria" | "caixa_suprimento" | "fornecedores_criar" | "compras_registrar" | "orcamento_criar" | "orcamento_adicional")
}

/// Id do registro que um comando de alteração/remoção atinge.
pub fn alvo_id(cmd: &str, args: &Value) -> Option<i64> {
    let altera = cmd.ends_with("_atualizar") || cmd.ends_with("_remover") || cmd.ends_with("_excluir")
        || cmd.ends_with("_substituir_itens") || cmd.ends_with("_alterar_status") || cmd.ends_with("_dar_acesso")
        || cmd.ends_with("_revogar_acesso") || cmd == "caixa_fechar" || cmd == "ordens_dados_veiculo_salvar"
        || matches!(cmd, "orcamento_enviar" | "orcamento_aprovar" | "orcamento_recusar" | "orcamento_cancelar" | "orcamento_converter_os");
    if altera { args.get("id").and_then(Value::as_i64) } else { None }
}

/// Hash da linha atual do alvo, para detectar conflito depois. None se não há alvo.
pub fn hash_base(conn: &Connection, cmd: &str, args: &Value) -> AppResult<Option<String>> {
    let (Some(t), Some(id)) = (tabela_do_comando(cmd), alvo_id(cmd, args)) else { return Ok(None) };
    Ok(linha_json(conn, t, id)?.map(|v| hash_linha(&v)))
}

/// No servidor: se o registro mudou desde a base que o cliente viu, devolve a versão atual (conflito).
pub fn verificar_base(conn: &Connection, cmd: &str, args: &Value, base_hash: &str) -> AppResult<Option<Value>> {
    let (Some(t), Some(id)) = (tabela_do_comando(cmd), alvo_id(cmd, args)) else { return Ok(None) };
    match linha_json(conn, t, id)? {
        Some(atual) if hash_linha(&atual) == base_hash => Ok(None),
        Some(atual) => Ok(Some(atual)),
        None => Ok(Some(Value::Null)),
    }
}

/// Reescreve ids locais (de registros criados offline) para os ids do servidor. `resolver` devolve o id do servidor.
pub fn remapear(cmd: &str, args: &Value, resolver: &dyn Fn(&str, i64) -> AppResult<Option<i64>>) -> AppResult<Value> {
    fn caminha(v: &Value, chave: Option<&str>, tabela_id: Option<&str>, res: &dyn Fn(&str, i64) -> AppResult<Option<i64>>) -> AppResult<Value> {
        Ok(match v {
            Value::Object(m) => {
                let mut out = serde_json::Map::new();
                for (k, val) in m {
                    let tabela = match k.as_str() {
                        "cliente_id" => Some("clientes"),
                        "veiculo_id" => Some("veiculos"),
                        "funcionario_id" => Some("funcionarios"),
                        "item_id" => Some("itens"),
                        "fornecedor_id" => Some("fornecedores"),
                        "id" => tabela_id,
                        _ => None,
                    };
                    let novo = match (tabela, val.as_i64()) {
                        (Some(t), Some(id)) => match res(t, id)? {
                            Some(s) => json!(s),
                            None => val.clone(),
                        },
                        _ => caminha(val, Some(k), None, res)?,
                    };
                    out.insert(k.clone(), novo);
                }
                Value::Object(out)
            }
            Value::Array(a) => Value::Array(a.iter().map(|x| caminha(x, chave, tabela_id, res)).collect::<AppResult<_>>()?),
            outro => outro.clone(),
        })
    }
    // `id` só é reescrito no topo do comando (é a chave do próprio registro atingido).
    let tabela_id = tabela_do_comando(cmd).filter(|_| alvo_id(cmd, args).is_some());
    caminha(args, None, tabela_id, resolver)
}

/// Momento do registro, no mesmo formato usado pelas ordens (RFC 3339 em UTC).
fn agora() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn arg<T: DeserializeOwned>(args: &Value, chave: &str) -> AppResult<T> {
    serde_json::from_value(args.get(chave).cloned().unwrap_or(Value::Null))
        .map_err(|e| AppError::Validation(vec![crate::error::FieldError { field: chave.into(), message: format!("dados inválidos: {e}") }]))
}

fn id(args: &Value) -> AppResult<i64> {
    arg::<i64>(args, "id")
}

fn ok<T: serde::Serialize>(v: T) -> AppResult<Value> {
    serde_json::to_value(v).map_err(|e| AppError::Database(e.to_string()))
}

/// Executa um comando de negócio. Não registra alteração: quem chama decide o status de sincronização.
pub fn executar(conn: &Connection, cmd: &str, args: &Value, ctx: &Contexto) -> AppResult<Value> {
    let alvo = perfil_do_alvo(conn, cmd, args)?;
    if !pode_executar(&ctx.perfil, cmd, args, alvo.as_deref()) {
        return Err(AppError::Conflict("Só o administrador pode mexer no acesso ou no cadastro de um administrador.".into()));
    }
    match cmd {
        "clientes_listar" => ok(clientes::listar(conn)?),
        "clientes_criar" => ok(clientes::criar(conn, &arg(args, "dados")?)?),
        "clientes_atualizar" => ok(clientes::atualizar(conn, id(args)?, &arg(args, "dados")?)?),
        "clientes_remover" => { clientes::remover(conn, id(args)?)?; Ok(Value::Null) }

        "veiculos_listar" => ok(veiculos::listar(conn)?),
        "veiculos_criar" => ok(veiculos::criar(conn, &arg(args, "dados")?)?),
        "veiculos_atualizar" => ok(veiculos::atualizar(conn, id(args)?, &arg(args, "dados")?)?),
        "veiculos_remover" => { veiculos::remover(conn, id(args)?)?; Ok(Value::Null) }
        "veiculos_foto_listar" => ok(fotos::listar(conn, arg(args, "veiculo_id")?)?),
        "veiculos_historico" => ok(veiculos::historico(conn, id(args)?)?),
        "ordens_dados_veiculo_salvar" => {
            let dados: veiculos::DadosAtendimento = arg(args, "dados")?;
            veiculos::salvar_dados_ordem(conn, id(args)?, &dados)?;
            ok(crate::ordens::buscar(conn, id(args)?)?)
        }
        "veiculos_foto_adicionar" => ok(fotos::adicionar(conn, &arg(args, "dados")?)?),
        "veiculos_foto_dados" => fotos::dados(conn, id(args)?),
        "veiculos_foto_remover" => { fotos::remover(conn, id(args)?)?; Ok(Value::Null) }

        "funcionarios_listar" => ok(funcionarios::listar(conn)?),
        "funcionarios_criar" => ok(funcionarios::criar(conn, &arg(args, "dados")?)?),
        "funcionarios_atualizar" => ok(funcionarios::atualizar(conn, id(args)?, &arg(args, "dados")?)?),
        "funcionarios_remover" => { funcionarios::remover(conn, id(args)?)?; Ok(Value::Null) }
        "funcionarios_dar_acesso" => ok(funcionarios::dar_acesso(conn, id(args)?, &arg(args, "dados")?)?),
        "funcionarios_revogar_acesso" => ok(funcionarios::revogar_acesso(conn, id(args)?)?),

        "itens_listar" => ok(itens::listar(conn)?),
        "itens_buscar_codigo" => ok(itens::buscar_por_codigo(conn, &arg::<String>(args, "codigo")?)?),
        "itens_criar" => ok(itens::criar(conn, &arg(args, "dados")?)?),
        "itens_atualizar" => ok(itens::atualizar(conn, id(args)?, &arg(args, "dados")?)?),
        "itens_remover" => { itens::remover(conn, id(args)?)?; Ok(Value::Null) }

        "ordens_listar" => ok(ordens::listar(conn)?),
        "ordens_buscar" => ok(ordens::buscar(conn, id(args)?)?),
        "ordens_criar" => ok(ordens::criar(conn, &arg(args, "dados")?)?),
        "ordens_substituir_itens" => {
            let itens: Vec<ordens::LinhaInput> = arg(args, "itens")?;
            ok(ordens::substituir_itens(conn, id(args)?, &itens)?)
        }
        "ordens_alterar_status" => {
            let s: String = arg(args, "status")?;
            let novo = ordens::Status::de_texto(&s).ok_or_else(|| AppError::Conflict(format!("status desconhecido: {s}")))?;
            if novo == ordens::Status::Concluida {
                // Concluir é receber: precisa de caixa aberto e de forma de pagamento.
                let pagamento: String = arg(args, "pagamento")?;
                crate::caixa::exigir_aberto(conn)?;
                ok(ordens::concluir_com_pagamento(conn, id(args)?, &pagamento)?)
            } else {
                ok(ordens::alterar_status(conn, id(args)?, novo)?)
            }
        }
        "ordens_excluir" => { ordens::excluir(conn, id(args)?)?; Ok(Value::Null) }

        "caixa_situacao" => {
            let mut s = crate::caixa::situacao(conn)?;
            // Caixa de outro operador: o funcionário não vê os valores, só de quem é e desde quando.
            let dono = s.sessao.as_ref().map(|x| x.aberto_por.clone());
            if ctx.perfil != ADMINISTRADOR && dono.is_some_and(|d| d != ctx.usuario) {
                s.recebimentos = Default::default();
                s.sangrias = 0.0;
                s.suprimentos = 0.0;
                s.dinheiro_esperado = 0.0;
                s.movimentos = Vec::new();
                s.de_outro_operador = true;
            }
            ok(s)
        }
        "caixa_historico" => ok(crate::caixa::historico(conn, arg::<i64>(args, "limite").unwrap_or(30))?),
        "caixa_abrir" => ok(crate::caixa::abrir(conn, arg(args, "valor_inicial").ok(), &ctx.usuario, &agora())?),
        "caixa_fechar" => {
            let contado: Option<f64> = arg(args, "valor_contado")?;
            let obs: Option<String> = arg(args, "observacao").ok();
            let id_caixa = id(args)?;
            // Quem fecha é quem abriu; o administrador pode fechar qualquer caixa, mas informa o motivo.
            if let Some(aberto) = crate::caixa::situacao(conn)?.sessao {
                if aberto.id == id_caixa && ctx.perfil != ADMINISTRADOR && aberto.aberto_por != ctx.usuario {
                    return Err(AppError::Conflict(format!(
                        "Este caixa foi aberto por {}. Só ele ou o administrador pode fechá-lo.", aberto.aberto_por
                    )));
                }
                if aberto.id == id_caixa && aberto.aberto_por != ctx.usuario && obs.as_deref().map(str::trim).unwrap_or("").is_empty() {
                    return Err(AppError::Conflict("Ao fechar o caixa de outro operador, informe o motivo na observação.".into()));
                }
            }
            ok(crate::caixa::fechar(conn, id_caixa, contado, obs, &ctx.usuario, &agora())?)
        }
        "caixa_sangria" | "caixa_suprimento" => {
            let tipo = if cmd == "caixa_sangria" { "sangria" } else { "suprimento" };
            let valor: Option<f64> = arg(args, "valor")?;
            let motivo: Option<String> = arg(args, "motivo").ok();
            ok(crate::caixa::registrar_movimento(conn, tipo, valor, motivo, &ctx.usuario, &agora())?)
        }
        "caixa_relatorio" => {
            let rel = crate::caixa::relatorio(conn, id(args)?)?;
            // Funcionário lê só o caixa que ele mesmo abriu.
            if ctx.perfil != ADMINISTRADOR && rel.sessao.aberto_por != ctx.usuario {
                return Err(AppError::Conflict("Só o administrador consulta o caixa de outro operador.".into()));
            }
            ok(rel)
        }
        "venda_balcao_criar" => ok(ordens::criar_balcao(conn, &arg(args, "dados")?)?),
        "fidelidade_saldo" => {
            let cliente: i64 = arg(args, "cliente_id")?;
            ok(json!({ "cliente_id": cliente, "saldo": crate::fidelidade::saldo(conn, cliente)? }))
        }
        "fidelidade_extrato" => ok(crate::fidelidade::extrato(conn, arg(args, "cliente_id")?)?),
        "orcamento_listar" => ok(orcamentos::listar(conn)?),
        "orcamento_buscar" => ok(orcamentos::buscar(conn, id(args)?)?),
        "orcamento_criar" => ok(orcamentos::criar(conn, &arg(args, "dados")?, &agora())?),
        "orcamento_enviar" => ok(orcamentos::enviar(conn, id(args)?, &agora())?),
        "orcamento_aprovar" => {
            let forma: Option<String> = arg(args, "forma")?;
            let observacao: Option<String> = arg(args, "observacao")?;
            ok(orcamentos::aprovar(conn, id(args)?, forma.as_deref().unwrap_or(""), observacao.as_deref(), &agora())?)
        }
        "orcamento_recusar" => ok(orcamentos::recusar(conn, id(args)?, &agora())?),
        "orcamento_cancelar" => ok(orcamentos::cancelar(conn, id(args)?, &agora())?),
        "orcamento_converter_os" => {
            let funcionario: Option<i64> = arg(args, "funcionario_id")?;
            ok(orcamentos::converter_os(conn, id(args)?, funcionario, &agora())?)
        }
        "orcamento_adicional" => ok(orcamentos::adicional(conn, &arg(args, "dados")?, &agora())?),
        // Relatórios: só leitura. Recebem `de` e `ate` (AAAA-MM-DD, inclusive).
        "relatorio_faturamento" | "relatorio_recebimentos" | "relatorio_contas" | "relatorio_lucro"
        | "relatorio_servicos" | "relatorio_produtividade" | "relatorio_estoque" | "relatorio_completo" => {
            let hoje = chrono::Local::now().date_naive();
            let periodo = || -> AppResult<crate::relatorios::Periodo> {
                crate::relatorios::Periodo::novo(&arg::<String>(args, "de")?, &arg::<String>(args, "ate")?)
            };
            match cmd {
                "relatorio_faturamento" => ok(crate::relatorios::faturamento(conn, &periodo()?)?),
                "relatorio_recebimentos" => ok(crate::relatorios::recebimentos(conn, &periodo()?)?),
                "relatorio_contas" => ok(crate::relatorios::contas(conn, hoje)?),
                "relatorio_lucro" => ok(crate::relatorios::lucro(conn, &periodo()?)?),
                "relatorio_servicos" => ok(crate::relatorios::servicos(conn, &periodo()?)?),
                "relatorio_produtividade" => ok(crate::relatorios::produtividade(conn, &periodo()?)?),
                "relatorio_estoque" => ok(crate::relatorios::estoque(conn, &periodo()?)?),
                _ => ok(crate::relatorios::completo(conn, &periodo()?, hoje)?),
            }
        }
        // Painel do administrador: só leitura, e só o administrador (a tela também esconde para funcionário).
        "painel_serie" => {
            if ctx.perfil != ADMINISTRADOR {
                return Err(AppError::Conflict("Só o administrador pode ver o painel.".into()));
            }
            ok(crate::painel::serie(conn, &arg::<String>(args, "de")?, &arg::<String>(args, "ate")?)?)
        }
        "dashboard_resumo" => ok(dashboard::resumo(conn)?),
        _ if COMANDOS_COMPRAS.contains(&cmd) => executar_compras(conn, cmd, args, &ctx.usuario),
        // Comandos financeiros (recebimentos, contas a receber/pagar, resumo) vivem em `financeiro`.
        c if c.starts_with("dispositivos_") || c.starts_with("pareamento_") => {
            crate::dispositivos::executar(conn, c, args, ctx)
        }
        _ => crate::financeiro::executar(conn, cmd, args, &ctx.usuario)
            .unwrap_or_else(|| Err(AppError::NotFound(format!("comando desconhecido: {cmd}")))),
    }
}

/// Id criado por um comando de criação (para mapear ids locais).
pub fn id_criado(resultado: &Value) -> Option<i64> {
    resultado.get("id").and_then(Value::as_i64)
}

// ---- Fornecedores e compras ----

/// Escritas deste bloco (entram na fila de sincronização como as demais).
const ESCRITA_COMPRAS: &[&str] = &["fornecedores_criar", "fornecedores_atualizar", "compras_registrar"];

/// Todos os comandos deste bloco (leituras e escritas).
const COMANDOS_COMPRAS: &[&str] = &[
    "fornecedores_listar", "fornecedores_criar", "fornecedores_atualizar", "fornecedores_buscar",
    "compras_registrar", "compras_listar", "compras_historico_fornecedor", "compras_historico_item",
];

fn executar_compras(conn: &Connection, cmd: &str, args: &Value, usuario: &str) -> AppResult<Value> {
    match cmd {
        "fornecedores_listar" => ok(crate::fornecedores::listar(conn)?),
        "fornecedores_criar" => ok(crate::fornecedores::criar(conn, &arg(args, "dados")?)?),
        "fornecedores_atualizar" => ok(crate::fornecedores::atualizar(conn, id(args)?, &arg(args, "dados")?)?),
        "fornecedores_buscar" => ok(crate::fornecedores::buscar_por_termo(conn, &arg::<String>(args, "termo").unwrap_or_default())?),
        "compras_registrar" => ok(crate::compras::registrar(conn, &arg(args, "dados")?, usuario)?),
        "compras_listar" => ok(crate::compras::listar(conn)?),
        "compras_historico_fornecedor" => ok(crate::compras::historico_fornecedor(conn, arg(args, "fornecedor_id")?)?),
        "compras_historico_item" => ok(crate::compras::historico_item(conn, arg(args, "item_id")?)?),
        _ => Err(AppError::NotFound(format!("comando desconhecido: {cmd}"))),
    }
}
