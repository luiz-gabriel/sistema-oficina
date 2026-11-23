//! Regras da aplicação: roteamento das gravações (local, servidor ou fila offline),
//! sessão, configuração e situação do sistema. Os comandos Tauri são finos e chamam estas funções.
use std::time::Duration;

use oficina_core::alteracoes::{self, Nova};
use oficina_core::registro;
use oficina_core::comandos::{self, Contexto};
use oficina_core::error::{AppError, AppResult};
use oficina_core::funcionarios::{self, AcessoInput, FuncionarioInput};
use oficina_core::{ativacao, config, hora, impressao, ordens, orcamentos, sessao};
use serde_json::{json, Value};

use crate::estado::{contexto_sistema, Estado};
use crate::{rede, sincronia};

/// Grava uma alteração conforme o modo de funcionamento deste computador.
pub fn escrever(est: &Estado, ctx: &Contexto, nome: &str, args_originais: &Value) -> AppResult<Value> {
    // Senha em texto puro vira hash já aqui (e é validada antes): fila, envio e servidor só recebem o hash.
    let preparado = alteracoes::preparar_args(nome, args_originais)?;
    let args = &preparado;
    let modo = est.modo();
    let token = est.rede.token.lock().unwrap().clone();
    // Mesmo identificador para o envio e para a fila: se a resposta se perder, o reenvio não é aplicado de novo.
    let uuid_op = alteracoes::novo_uuid();
    if modo == "cliente" && !est.offline() && token.is_some() {
        if let Ok(mut canal) = sincronia::conectar_servidor(est) {
            // Pendências antigas vão antes desta alteração. Se elas não puderem ser enviadas, esta também vai para a fila.
            if sincronia::enviar_pendentes(est, &mut canal).is_ok() {
                let pedido = json!({
                    "t": "cmd", "nome": nome, "args": args, "token": token, "uuid": uuid_op,
                    "computador": est.computador, "base": null
                });
                if let Ok(resp) = canal.pedir(&pedido) {
                    if resp["ok"] == json!(true) {
                        let valor = resp["valor"].clone();
                        let _ = sincronia::puxar(est, &mut canal);
                        return Ok(valor);
                    }
                    return Err(AppError::de_json(&resp["erro"]));
                }
            }
        }
        // Sem servidor: a alteração fica na fila e é enviada quando a rede voltar.
        return escrever_offline(est, ctx, nome, args, &uuid_op);
    }
    // Cliente offline, ou servidor isolado (sem outro computador alcançável): a alteração fica pendente, nunca sincronizada.
    if modo == "cliente" || (modo == "servidor" && est.offline()) {
        return escrever_offline(est, ctx, nome, args, &uuid_op);
    }
    let c = est.db()?;
    let valor = comandos::executar(&c, nome, args, ctx)?;
    let status = if modo == "local" { "local" } else { "sincronizado" };
    if comandos::e_escrita(nome) {
        alteracoes::registrar(&c, &Nova {
            computador: &est.computador,
            usuario: &ctx.usuario,
            comando: nome,
            args,
            tabela: comandos::tabela_do_comando(nome),
            id_local: comandos::id_criado(&valor).or(comandos::alvo_id(nome, args)),
            base_hash: None,
            status,
        })?;
    }
    Ok(valor)
}

fn escrever_offline(est: &Estado, ctx: &Contexto, nome: &str, args: &Value, uuid_op: &str) -> AppResult<Value> {
    let c = est.db()?;
    let tabela = comandos::tabela_do_comando(nome);
    // Registro que só existe neste computador: não há versão do servidor para comparar.
    let alvo_local = match (tabela, comandos::alvo_id(nome, args)) {
        (Some(t), Some(id)) => alteracoes::situacao_id(&c, t, id)?.is_some(),
        _ => false,
    };
    let base = if alvo_local { None } else { comandos::hash_base(&c, nome, args)? };
    let valor = comandos::executar(&c, nome, args, ctx)?;
    let id_local = if comandos::cria_registro(nome) {
        let id = comandos::id_criado(&valor);
        if let (Some(t), Some(i)) = (tabela, id) {
            alteracoes::registrar_id_local(&c, t, i)?;
        }
        id
    } else {
        comandos::alvo_id(nome, args)
    };
    alteracoes::registrar_com_uuid(&c, &Nova {
        computador: &est.computador,
        usuario: &ctx.usuario,
        comando: nome,
        args,
        tabela,
        id_local,
        base_hash: base.as_deref(),
        status: "pendente",
    }, uuid_op)?;
    Ok(valor)
}

/// Comando vindo da tela.
pub fn comando(est: &Estado, nome: &str, args: &Value) -> AppResult<Value> {
    let ctx = est.sessao.lock().unwrap().clone().ok_or_else(|| AppError::Conflict("Faça login para continuar.".into()))?;
    if !comandos::e_escrita(nome) {
        let c = est.db()?;
        return comandos::executar(&c, nome, args, &ctx);
    }
    let valor = escrever(est, &ctx, nome, args)?;
    anotar(est, &ctx.usuario, nome, args);
    Ok(valor)
}

pub fn estado_sistema(est: &Estado) -> AppResult<Value> {
    let (ativado, tem_admin, nome_logado, pendentes) = {
        let c = est.db()?;
        let ativado = ativacao::esta_ativado(&c)?;
        let tem_admin = funcionarios::existe_administrador(&c)?;
        let logado = est.sessao.lock().unwrap().clone();
        let nome = match &logado {
            Some(s) => c
                .query_row("SELECT nome FROM funcionarios WHERE login = ?1", [&s.usuario], |r| r.get::<_, String>(0))
                .ok(),
            None => None,
        };
        (ativado, tem_admin, logado.map(|s| (s, nome)), alteracoes::contar_pendentes(&c)?)
    };
    // Última sessão guardada, para a tela de entrada oferecer "Continuar como" (só com ninguém logado).
    let ultima_sessao = if nome_logado.is_none() {
        let c = est.db()?;
        sessao::para_continuar(&c, chrono::Utc::now())?.map(|(f, minutos)| {
            json!({"usuario": f.login.clone().unwrap_or_default(), "nome": f.nome, "perfil": f.perfil, "ha_minutos": minutos})
        })
    } else {
        None
    };
    let logado = nome_logado.map(|(s, nome)| json!({"usuario": s.usuario, "perfil": s.perfil, "nome": nome}));
    Ok(json!({
        "ultima_sessao": ultima_sessao,
        "ativado": ativado,
        "tem_admin": tem_admin,
        "modo": est.modo(),
        "offline": est.offline(),
        "papel": est.papel(),
        "computador": est.computador,
        "pendentes": pendentes,
        "servidor_addr": est.servidor_addr(),
        "logado": logado,
        "alerta": alerta(est, pendentes)?,
    }))
}

fn alerta(est: &Estado, pendentes: i64) -> AppResult<Value> {
    let modo = est.modo();
    let texto = match modo.as_str() {
        "cliente" if est.offline() => Some(("offline", format!(
            "Trabalhando em modo offline. {pendentes} alteração(ões) ainda não foram sincronizadas. Elas serão enviadas quando a rede voltar."))),
        "cliente" if est.papel() == "sem_servidor" => Some(("sem_servidor", "Sem comunicação com o servidor da oficina. A sincronização está indisponível; você ainda pode consultar os dados.".into())),
        "servidor" if est.offline() => Some(("isolado", "Este computador está isolado da rede e não atua como servidor. As alterações serão enviadas quando a rede voltar.".into())),
        "servidor" if est.rede.houve_clientes.load(std::sync::atomic::Ordering::Relaxed) && est.clientes_recentes(Duration::from_secs(20)) == 0 => Some(("sem_clientes", "Este computador é o servidor, mas não consegue se comunicar com os outros computadores. As consultas continuam funcionando.".into())),
        _ => None,
    };
    Ok(match texto {
        Some((tipo, t)) => json!({"tipo": tipo, "texto": t}),
        None => Value::Null,
    })
}

/// Ativa com a hora de uma fonte atômica (SNTP). Sem internet, usa o relógio local e avisa a tela.
pub fn ativar(est: &Estado, codigo: &str) -> AppResult<Value> {
    // A consulta de hora pode levar alguns segundos: é feita ANTES de travar o banco.
    let hora_atomica = hora::obter_hora(&hora::SERVIDORES, hora::TIMEOUT);
    let c = est.db()?;
    let escolhida = ativacao::ativar_com(&c, codigo, move || hora_atomica)?;
    Ok(escolhida.para_json())
}

pub fn criar_administrador(est: &Estado, nome: &str, login: &str, senha: &str) -> AppResult<()> {
    {
        let c = est.db()?;
        if !ativacao::esta_ativado(&c)? {
            return Err(AppError::Conflict("Ative o sistema antes de criar o administrador.".into()));
        }
        if funcionarios::existe_administrador(&c)? {
            return Err(AppError::Conflict("Já existe um administrador.".into()));
        }
    }
    if est.modo() == "cliente" {
        return Err(AppError::Conflict("Este computador é cliente: o administrador é criado no servidor.".into()));
    }
    let sistema = contexto_sistema();
    let f = escrever(est, &sistema, "funcionarios_criar", &json!({"dados": FuncionarioInput {
        nome: Some(nome.into()), cargo: Some("Administrador".into()), telefone: None,
    }}))?;
    escrever(est, &sistema, "funcionarios_dar_acesso", &json!({"id": f["id"], "dados": AcessoInput {
        login: Some(login.into()), senha: Some(senha.into()), senha_hash: None, perfil: Some("administrador".into()),
    }}))?;
    Ok(())
}

pub fn login(est: &Estado, login: &str, senha: &str) -> AppResult<Value> {
    // O banco fica preso só enquanto confere a senha; o registro abre o banco de novo, então soltamos antes.
    let entrada = {
        let c = est.db()?;
        funcionarios::entrar(&c, login, senha, chrono::Utc::now())?
    };
    let quem = login.trim().to_lowercase();
    match entrada {
        funcionarios::Entrada::Ok(f) => {
            let usuario = f.login.clone().unwrap_or_default();
            let ctx = Contexto { usuario: usuario.clone(), perfil: f.perfil.clone(), computador: est.computador.clone() };
            *est.sessao.lock().unwrap() = Some(ctx);
            // Guarda o login para a próxima abertura do sistema (só usuário, nome, perfil e horário).
            if let Err(e) = est.db().and_then(|c| sessao::guardar(&c, &f, chrono::Utc::now())) {
                eprintln!("aviso: não consegui guardar a sessão: {e}");
            }
            pedir_token(est, login, senha);
            // Com o token novo, as alterações pendentes deste usuário já podem ser enviadas.
            if est.modo() == "cliente" && !est.offline() {
                let _ = sincronia::sincronizar(est);
            }
            anotar(est, &usuario, "entrou no sistema", &json!({}));
            Ok(json!({"usuario": usuario, "nome": f.nome, "perfil": f.perfil}))
        }
        funcionarios::Entrada::Falhou => {
            anotar(est, &quem, "errou a senha ao entrar", &json!({}));
            Err(AppError::Conflict("Login ou senha incorretos. Depois de 3 erros seguidos, o acesso fica bloqueado por 5 minutos.".into()))
        }
        funcionarios::Entrada::Bloqueado { ate } => {
            anotar(est, &quem, "acesso bloqueado após 3 tentativas erradas", &json!({}));
            let hora = chrono::DateTime::parse_from_rfc3339(&ate)
                .map(|d| d.with_timezone(&chrono::Local).format("%H:%M").to_string())
                .unwrap_or_default();
            Err(AppError::Conflict(format!(
                "Acesso bloqueado por excesso de tentativas erradas. Tente de novo depois das {hora} (5 minutos)."
            )))
        }
    }
}

/// No cliente com servidor alcançável, pega o token da sessão. Sem servidor, o login continua valendo localmente.
fn pedir_token(est: &Estado, login: &str, senha: &str) {
    if est.modo() != "cliente" || est.offline() {
        return;
    }
    if let Ok(mut canal) = sincronia::conectar_servidor(est) {
        if let Ok(resp) = rede::entrar(&mut canal, login, senha) {
            if resp["ok"] == json!(true) {
                *est.rede.token.lock().unwrap() = resp["token"].as_str().map(String::from);
            }
        }
    }
}

pub fn logout(est: &Estado) {
    let usuario = usuario_atual(est);
    let token = est.rede.token.lock().unwrap().take();
    if let (Some(t), true) = (token, est.modo() == "cliente" && !est.offline()) {
        if let Ok(mut canal) = sincronia::conectar_servidor(est) {
            let _ = rede::sair(&mut canal, &t);
        }
    }
    *est.sessao.lock().unwrap() = None;
    if let Err(e) = est.db().and_then(|c| sessao::apagar(&c)) {
        eprintln!("aviso: não consegui apagar a sessão guardada: {e}");
    }
    anotar(est, &usuario, "saiu do sistema", &json!({}));
}

/// Quem está logado agora (ou "sistema" antes de qualquer login).
fn usuario_atual(est: &Estado) -> String {
    est.sessao.lock().unwrap().as_ref().map(|s| s.usuario.clone()).unwrap_or_else(|| "sistema".into())
}

/// Anota no registro de atividades. Se a anotação falhar, a ação já feita continua valendo; o aviso vai para o terminal.
fn anotar(est: &Estado, usuario: &str, acao: &str, detalhe: &Value) {
    let resultado = est.db().and_then(|c| registro::registrar(&c, usuario, &est.computador, acao, &registro::resumir(detalhe)));
    if let Err(e) = resultado {
        eprintln!("aviso: não consegui registrar a atividade '{acao}': {e}");
    }
}

/// Registro de atividades: só o administrador consulta.
pub fn registro_listar(est: &Estado, limite: i64) -> AppResult<Value> {
    exige_registro(est)?;
    let c = est.db()?;
    Ok(json!(registro::listar(&c, limite)?))
}

/// Logo da empresa (imagem em data URL). Fica neste computador; qualquer tela pode mostrá-la.
pub fn logo_obter(est: &Estado) -> AppResult<String> {
    est.get("logo", "")
}

pub fn logo_salvar(est: &Estado, dados: &str) -> AppResult<()> {
    exige_administrador(est)?;
    let aceitos = ["data:image/png;base64,", "data:image/jpeg;base64,", "data:image/webp;base64,"];
    if dados.is_empty() {
        est.definir("logo", "")?;
        anotar(est, &usuario_atual(est), "removeu a logo da empresa", &json!({}));
        return Ok(());
    }
    if !aceitos.iter().any(|p| dados.starts_with(p)) {
        return Err(AppError::Validation(vec![oficina_core::error::FieldError {
            field: "logo".into(), message: "Escolha uma imagem PNG, JPG ou WEBP.".into(),
        }]));
    }
    // Limite de tamanho: a logo vai para o banco e para a tela inicial, então precisa ser leve.
    if dados.len() > 400_000 {
        return Err(AppError::Validation(vec![oficina_core::error::FieldError {
            field: "logo".into(), message: "A imagem é grande demais. Use uma imagem menor (até cerca de 300 KB).".into(),
        }]));
    }
    est.definir("logo", dados)?;
    anotar(est, &usuario_atual(est), "trocou a logo da empresa", &json!({"tamanho_kb": dados.len() / 1024}));
    Ok(())
}

/// Configurações da oficina: só o administrador (regra em `comandos::pode_configurar`).
pub fn exige_administrador(est: &Estado) -> AppResult<()> {
    let tem_admin = {
        let c = est.db()?;
        funcionarios::existe_administrador(&c)?
    };
    if !tem_admin {
        // Configuração inicial (antes do primeiro administrador) é liberada.
        return Ok(());
    }
    match est.sessao.lock().unwrap().as_ref() {
        Some(s) if comandos::pode_configurar(&s.perfil) => Ok(()),
        _ => Err(AppError::Conflict("Apenas o administrador pode alterar a configuração.".into())),
    }
}

/// Registro de atividades: só o administrador logado (sem a exceção da configuração inicial).
fn exige_registro(est: &Estado) -> AppResult<()> {
    match est.sessao.lock().unwrap().as_ref() {
        Some(s) if comandos::pode_ver_registro(&s.perfil) => Ok(()),
        _ => Err(AppError::Conflict("Apenas o administrador pode ver o registro de atividades.".into())),
    }
}

pub fn config_obter(est: &Estado) -> AppResult<Value> {
    // Lê tudo do banco num bloco curto; depois chama os demais métodos (que também travam o banco).
    let (modo, dispositivo, ativado_em, ultimo_backup, fonte_hora, desvio_hora, situacao_rede) = {
        let c = est.db()?;
        (
            config::obter_ou(&c, "modo", "")?,
            config::obter_ou(&c, "dispositivo_id", "")?,
            config::obter_ou(&c, "ativado_em", "")?,
            config::obter_ou(&c, "ultimo_backup", "")?,
            config::obter_ou(&c, "ativacao_fonte", "")?,
            config::obter_ou(&c, "ativacao_desvio_segundos", "")?,
            config::obter_ou(&c, "situacao_rede", "")?,
        )
    };
    Ok(json!({
        "situacao_rede": situacao_rede,
        "fonte_hora": fonte_hora,
        "desvio_hora_segundos": desvio_hora,
        "modo": modo,
        "servidor_addr": est.servidor_addr(),
        "offline": est.offline(),
        "papel": est.papel(),
        "computador": est.computador,
        "porta": est.porta,
        "ip_local": rede::meu_ip().map(|i| i.to_string()),
        "dispositivo": dispositivo,
        "ativado_em": ativado_em,
        "ultimo_backup": ultimo_backup,
    }))
}

/// Escolhe o modo. Cliente exige o endereço do servidor e já copia os dados dele.
pub fn config_modo(est: &Estado, modo: &str, endereco: Option<String>) -> AppResult<()> {
    let usuario = usuario_atual(est);
    let resultado = config_modo_interno(est, modo, endereco);
    if resultado.is_ok() {
        anotar(est, &usuario, "configurou o modo de funcionamento", &json!({"modo": modo}));
    }
    resultado
}

fn config_modo_interno(est: &Estado, modo: &str, endereco: Option<String>) -> AppResult<()> {
    exige_administrador(est)?;
    match modo {
        "local" | "servidor" => {
            if modo == "servidor" && est.epoca() == 0 {
                // Primeiro servidor da licença: começa a época 1 (ver `rede_regras::proxima_epoca`).
                est.definir("epoca", "1")?;
            }
            est.definir("modo", modo)?;
            est.definir("offline", "0")?;
            est.definir("servidor_addr", "")?;
            Ok(())
        }
        "cliente" => {
            let addr = endereco.filter(|e| !e.trim().is_empty())
                .ok_or_else(|| AppError::Validation(vec![oficina_core::error::FieldError {
                    field: "endereco".into(), message: "informe o endereço do servidor (ex.: 192.168.0.10:7878)".into(),
                }]))?;
            est.definir("servidor_addr", addr.trim())?;
            sincronia::sincronizar(est).map_err(|e| {
                let _ = est.definir("servidor_addr", "");
                e
            })?;
            est.definir("modo", "cliente")?;
            est.definir("offline", "0")?;
            est.rede.conectado.store(true, std::sync::atomic::Ordering::Relaxed);
            Ok(())
        }
        _ => Err(AppError::Conflict("Modo desconhecido.".into())),
    }
}

pub fn rede_buscar(est: &Estado) -> AppResult<Value> {
    let id = est.identidade()?.ok_or_else(|| AppError::Conflict("Ative o sistema primeiro.".into()))?;
    let encontrados = rede::descobrir(&est.portas_descoberta(), &id, &[]);
    let lista: Vec<Value> = encontrados
        .into_iter()
        .filter(|i| i.dispositivo != id.dispositivo)
        .map(|i| json!({"endereco": i.endereco, "computador": i.computador, "papel": i.papel}))
        .collect();
    Ok(json!(lista))
}

pub fn sincronizar_agora(est: &Estado) -> AppResult<Value> {
    if est.modo() == "cliente" {
        sincronia::sincronizar(est)?;
        est.definir("offline", "0")?;
    }
    let c = est.db()?;
    Ok(json!({"pendentes": alteracoes::contar_pendentes(&c)?}))
}

pub fn trabalhar_offline(est: &Estado, ligar: bool) -> AppResult<()> {
    let usuario = usuario_atual(est);
    let resultado = trabalhar_offline_interno(est, ligar);
    if resultado.is_ok() {
        anotar(est, &usuario, if ligar { "ligou o modo offline" } else { "voltou a sincronizar" }, &json!({}));
    }
    resultado
}

fn trabalhar_offline_interno(est: &Estado, ligar: bool) -> AppResult<()> {
    est.definir("offline", if ligar { "1" } else { "0" })?;
    if !ligar && est.modo() == "cliente" {
        let _ = sincronia::sincronizar(est);
    }
    Ok(())
}

pub fn pendencias(est: &Estado) -> AppResult<Value> {
    let c = est.db()?;
    let lista = alteracoes::listar(&c, &["pendente", "conflito", "erro"])?;
    Ok(json!(lista.into_iter().map(|mut a| { a.args = oficina_core::registro::sem_segredos(&a.args); a }).collect::<Vec<_>>()))
}

pub fn historico(est: &Estado, limite: i64) -> AppResult<Value> {
    exige_administrador(est)?;
    let c = est.db()?;
    let lista = alteracoes::historico(&c, limite)?;
    Ok(json!(lista.into_iter().map(|mut a| { a.args = oficina_core::registro::sem_segredos(&a.args); a }).collect::<Vec<_>>()))
}

/// Versão local e versão do servidor de uma alteração em conflito, lado a lado (sem senha).
pub fn versoes_conflito(est: &Estado, uuid: &str) -> AppResult<Value> {
    let c = est.db()?;
    Ok(json!(alteracoes::versoes_do_conflito(&c, uuid)?))
}

pub fn resolver_conflito(est: &Estado, uuid: &str, escolha: &str) -> AppResult<()> {
    let usuario = usuario_atual(est);
    let resultado = resolver_conflito_interno(est, uuid, escolha);
    if resultado.is_ok() {
        anotar(est, &usuario, "resolveu conflito de sincronização", &json!({"escolha": escolha}));
    }
    resultado
}

fn resolver_conflito_interno(est: &Estado, uuid: &str, escolha: &str) -> AppResult<()> {
    if escolha != "servidor" && escolha != "local" {
        return Err(AppError::Conflict("Escolha 'servidor' ou 'local'.".into()));
    }
    // Ler o modo ANTES de pegar o banco: `modo()` também usa o mesmo lock e travaria a thread.
    let modo = est.modo();
    {
        let c = est.db()?;
        alteracoes::resolver(&c, uuid, escolha)?;
        // O servidor não reenvia a própria fila: a escolha "local" é aplicada aqui mesmo.
        if modo == "servidor" && escolha == "local" {
            if let Some(item) = alteracoes::listar(&c, &["pendente"])?.into_iter().find(|a| a.uuid == uuid) {
                let ctx = Contexto {
                    usuario: item.usuario.clone(),
                    perfil: "administrador".into(),
                    computador: est.computador.clone(),
                };
                comandos::executar(&c, &item.comando, &item.args, &ctx)?;
                alteracoes::atualizar(&c, uuid, "sincronizado", None, None)?;
            }
        }
    }
    if modo == "cliente" && !est.offline() {
        let _ = sincronia::sincronizar(est);
    }
    Ok(())
}

// ---- Impressora térmica deste computador (fica na configuração local, não é sincronizada) ----

fn destino_configurado(est: &Estado) -> AppResult<impressao::Destino> {
    let texto = est.get("impressora_destino", "")?;
    if texto.trim().is_empty() {
        return Err(AppError::Conflict("Nenhuma impressora configurada. Escolha em Configurações > Impressora térmica.".into()));
    }
    impressao::destino(&texto)
}

pub fn impressora_atual(est: &Estado) -> AppResult<String> {
    est.get("impressora_destino", "")
}

pub fn impressora_salvar(est: &Estado, texto: &str) -> AppResult<()> {
    exige_administrador(est)?;
    let usuario = usuario_atual(est);
    let resultado = impressora_salvar_interno(est, texto);
    if resultado.is_ok() {
        anotar(est, &usuario, "configurou a impressora térmica", &json!({"destino": texto.trim()}));
    }
    resultado
}

fn impressora_salvar_interno(est: &Estado, texto: &str) -> AppResult<()> {
    if texto.trim().is_empty() {
        // Vazio desliga a impressora.
        return est.definir("impressora_destino", "");
    }
    impressao::destino(texto)?;
    est.definir("impressora_destino", texto.trim())
}

pub fn impressora_testar(est: &Estado) -> AppResult<()> {
    exige_administrador(est)?;
    let dest = destino_configurado(est)?;
    impressao::enviar(&dest, &impressao::teste()?)
}

/// Imprime o orçamento (ou aditivo) como ORÇAMENTO, nunca como documento fiscal.
pub fn imprimir_orcamento(est: &Estado, id: i64) -> AppResult<()> {
    let orc = {
        let c = est.db()?;
        orcamentos::buscar(&c, id)?
    };
    let dest = destino_configurado(est)?;
    impressao::enviar(&dest, &impressao::cupom_orcamento(&orc)?)
}

pub fn imprimir_os(est: &Estado, id: i64) -> AppResult<()> {
    // Lê a ordem e solta o banco antes de falar com a impressora (que pode demorar).
    let os = {
        let c = est.db()?;
        ordens::buscar(&c, id)?
    };
    let dest = destino_configurado(est)?;
    impressao::enviar(&dest, &impressao::cupom_os(&os)?)
}
