//! Lado cliente: envia alterações pendentes, atualiza a cópia local, procura servidor e faz eleição.
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use oficina_core::alteracoes;
use oficina_core::comandos;
use oficina_core::error::{AppError, AppResult};
use oficina_core::rede_regras::{self, Decisao};
use oficina_core::sync::{self, SituacaoServidor};
use serde_json::json;

use crate::estado::Estado;
use crate::rede::{self, Canal, Info};

/// Mensagem do servidor quando o token deixou de valer (ex.: o servidor mudou ou reiniciou).
/// A alteração não é perdida: fica pendente e só é enviada depois de um novo login.
pub const MSG_SESSAO_EXPIRADA: &str = "Sessão expirada no servidor. Entre de novo para enviar as alterações pendentes.";

pub fn erro_rede(e: std::io::Error) -> AppError {
    AppError::Offline(format!("Sem comunicação com o servidor da oficina ({e})."))
}

pub fn conectar_servidor(est: &Estado) -> AppResult<Canal> {
    let addr: SocketAddr = est.endereco_servidor()?;
    let id = est.identidade()?.ok_or_else(|| AppError::Conflict("Ative o sistema primeiro.".into()))?;
    let canal = rede::conectar(addr, &id, Duration::from_secs(20)).map_err(erro_rede)?;
    // Servidor com dados de época anterior aos deste computador (ex.: principal que voltou de uma queda): não serve.
    if !rede_regras::servidor_aceitavel(id.epoca, canal.info.epoca) {
        return Err(AppError::Offline("O computador conectado tem dados mais antigos que os deste. Aguardando o servidor atual.".into()));
    }
    Ok(canal)
}

/// Envia ao servidor as alterações feitas offline, na ordem em que aconteceram.
pub fn enviar_pendentes(est: &Estado, canal: &mut Canal) -> AppResult<()> {
    let pendentes = {
        let c = est.db()?;
        // Filas antigas podem ter senha em texto puro: troca por hash antes de qualquer envio.
        alteracoes::migrar_senhas_em_claro(&c)?;
        alteracoes::listar(&c, &["pendente"])?
    };
    // Só envia com a sessão do próprio usuário (token emitido pelo servidor no login online).
    // Alterações de outro usuário, ou feitas sem login online, ficam na fila até ele entrar online.
    let (quem, token) = {
        let usuario = est.sessao.lock().unwrap().as_ref().map(|s| s.usuario.clone());
        (usuario, est.rede.token.lock().unwrap().clone())
    };
    for item in pendentes {
        if token.is_none() || quem.as_deref() != Some(item.usuario.as_str()) {
            continue;
        }
        let args = {
            let c = est.db()?;
            alteracoes::preparar_envio(&c, &item)
        };
        let args = match args {
            Ok(Some(a)) => a,
            // Depende de um registro criado offline que o servidor ainda não aceitou: espera, sem mudar de estado.
            Ok(None) => continue,
            Err(e) => {
                let c = est.db()?;
                alteracoes::atualizar(&c, &item.uuid, "erro", Some(&e.to_string()), None)?;
                continue;
            }
        };
        let base = if item.forcar { None } else { item.base_hash.clone() };
        // O identificador da alteração faz o servidor reconhecer um reenvio (ex.: a resposta se perdeu no meio).
        let resp = canal
            .pedir(&json!({
                "t": "cmd", "nome": item.comando, "args": args, "token": token,
                "computador": item.computador, "base": base, "uuid": item.uuid
            }))
            .map_err(erro_rede)?;
        let c = est.db()?;
        if resp["sessao_invalida"] == json!(true) {
            // Token antigo: limpa e para. Os itens continuam pendentes e são enviados no próximo login.
            *est.rede.token.lock().unwrap() = None;
            return Err(AppError::Conflict(MSG_SESSAO_EXPIRADA.into()));
        }
        if resp["ok"] == json!(true) {
            if comandos::cria_registro(&item.comando) {
                if let (Some(t), Some(idl), Some(ids)) = (item.tabela.as_deref(), item.id_local, resp["valor"]["id"].as_i64()) {
                    alteracoes::definir_id_servidor(&c, t, idl, ids)?;
                }
            }
            alteracoes::atualizar(&c, &item.uuid, "sincronizado", None, None)?;
        } else if let Some(atual) = resp.get("conflito") {
            let conflito = json!({"servidor": atual, "local": item.args, "comando": item.comando});
            alteracoes::atualizar(&c, &item.uuid, "conflito", None, Some(&conflito))?;
        } else {
            let e = AppError::de_json(&resp["erro"]);
            alteracoes::atualizar(&c, &item.uuid, "erro", Some(&e.to_string()), None)?;
        }
    }
    let c = est.db()?;
    if alteracoes::contar_pendentes(&c)? == 0 {
        alteracoes::limpar_ids_locais(&c)?;
    }
    Ok(())
}

/// Substitui a cópia local pelos dados do servidor, mantendo por cima as alterações que o servidor ainda não aceitou.
pub fn puxar(est: &Estado, canal: &mut Canal) -> AppResult<()> {
    // Sem usuário logado neste computador não há sessão: o servidor não entrega os dados. Não é erro de rede.
    let Some(token) = est.rede.token.lock().unwrap().clone() else { return Ok(()) };
    let resp = canal.pedir(&json!({"t": "snapshot", "token": token})).map_err(erro_rede)?;
    if resp["sessao_invalida"] == json!(true) {
        *est.rede.token.lock().unwrap() = None;
        return Err(AppError::Conflict(MSG_SESSAO_EXPIRADA.into()));
    }
    if resp["ok"] != json!(true) {
        return Err(AppError::de_json(&resp["erro"]));
    }
    {
        let mut c = est.db()?;
        sync::aplicar_snapshot_preservando_fila(&mut c, &resp["dados"])?;
    }
    // Passa a ter os dados da época do servidor de onde copiou.
    est.definir("epoca", &canal.info.epoca.to_string())
}

/// Sincronização completa: envia pendências e atualiza a cópia.
/// Token expirado não é falha de rede: o servidor respondeu, então a cópia é atualizada mesmo assim.
pub fn sincronizar(est: &Estado) -> AppResult<()> {
    let mut canal = conectar_servidor(est)?;
    match enviar_pendentes(est, &mut canal) {
        Ok(()) => {}
        Err(AppError::Conflict(m)) if m == MSG_SESSAO_EXPIRADA => {}
        Err(e) => return Err(e),
    }
    match puxar(est, &mut canal) {
        Ok(()) => {}
        // O servidor respondeu; só o token deixou de valer. Não conta como falha de rede.
        Err(AppError::Conflict(m)) if m == MSG_SESSAO_EXPIRADA => {}
        Err(e) => return Err(e),
    }
    est.rede.conectado.store(true, Ordering::Relaxed);
    est.rede.falhas.store(0, Ordering::Relaxed);
    if est.get("situacao_rede", "").unwrap_or_default() != "" {
        let _ = est.definir("situacao_rede", "");
    }
    Ok(())
}

/// Procura um servidor que o cliente aceite (época igual ou mais recente que a deste computador).
fn procurar_servidor(est: &Estado) -> Option<Info> {
    let id = est.identidade().ok()??;
    let extras: Vec<SocketAddr> = est.endereco_servidor().ok().into_iter().collect();
    rede::descobrir(&est.portas_descoberta(), &id, &extras)
        .into_iter()
        .find(|i| i.papel == "servidor" && i.dispositivo != id.dispositivo && rede_regras::servidor_aceitavel(id.epoca, i.epoca))
}

/// Eleição: a regra (quórum, época e antiguidade) fica em `rede_regras`. Ver `rede::avaliar_eleicao`.
fn eleicao(est: &Estado) {
    let Ok(Some(id)) = est.identidade() else { return };
    let peers = rede::descobrir(&est.portas_descoberta(), &id, &[]);
    let (situacao, nova_epoca) = rede::avaliar_eleicao(est, &peers);
    match situacao {
        SituacaoServidor::Promover => {
            let _ = est.definir("epoca", &nova_epoca.to_string());
            let _ = est.definir("modo", "servidor");
            let _ = est.definir("offline", "0");
            let _ = est.definir("servidor_addr", "");
            let _ = est.definir("situacao_rede", "");
            est.rede.conectado.store(false, Ordering::Relaxed);
        }
        SituacaoServidor::ServidorAtivo { .. } => {
            let _ = est.definir("situacao_rede", "servidor_ativo");
        }
        // Sem condições seguras para escolher servidor: a tela mostra o aviso com o motivo.
        SituacaoServidor::SemServidorSeguro { motivo } => {
            let _ = est.definir("situacao_rede", "sem_servidor_seguro");
            let _ = est.definir("situacao_rede_motivo", &motivo);
        }
    }
}

/// Servidor que encontra computador com dados mais recentes (ou outro servidor com prioridade) passa a cliente.
/// Assim, o principal antigo que volta de uma queda não assume de novo por cima de quem tem os dados atuais.
fn checar_servidor(est: &Estado) {
    let Ok(Some(id)) = est.identidade() else { return };
    let peers = rede::descobrir(&est.portas_descoberta(), &id, &[]);
    if rede::avaliar_servidor(est, &peers) != Decisao::Ceder {
        return;
    }
    let novo = peers
        .iter()
        .find(|i| i.papel == "servidor" && i.dispositivo != id.dispositivo && rede_regras::servidor_aceitavel(id.epoca, i.epoca))
        .map(|i| i.endereco.clone())
        .unwrap_or_default();
    let _ = est.definir("modo", "cliente");
    let _ = est.definir("offline", "0");
    let _ = est.definir("servidor_addr", &novo);
    est.rede.conectado.store(false, Ordering::Relaxed);
}

fn busca_permitida(est: &Estado, intervalo: Duration) -> bool {
    let mut u = est.rede.ultima_busca.lock().unwrap();
    match *u {
        Some(t) if t.elapsed() < intervalo => false,
        _ => {
            *u = Some(Instant::now());
            true
        }
    }
}

fn ciclo(est: &Estado) {
    match est.modo().as_str() {
        "servidor" => {
            // Ao ligar e depois de tempos em tempos: se há computador com dados mais recentes, cede.
            if busca_permitida(est, Duration::from_secs(15)) {
                checar_servidor(est);
            }
        }
        "cliente" => ciclo_cliente(est),
        _ => {}
    }
}

fn ciclo_cliente(est: &Estado) {
    if est.offline() {
        // Trabalhando offline: quando a rede voltar, envia as pendências antes de voltar ao normal.
        if let Ok(()) = sincronizar(est) {
            let _ = est.definir("offline", "0");
        }
        return;
    }
    match sincronizar(est) {
        Ok(()) => {}
        Err(_) => {
            est.rede.conectado.store(false, Ordering::Relaxed);
            let n = est.rede.falhas.fetch_add(1, Ordering::Relaxed) + 1;
            if n == 2 && busca_permitida(est, Duration::from_secs(5)) {
                if let Some(s) = procurar_servidor(est) {
                    let _ = est.definir("servidor_addr", &s.endereco);
                    est.rede.falhas.store(0, Ordering::Relaxed);
                }
            } else if n >= 6 {
                eleicao(est);
                est.rede.falhas.store(0, Ordering::Relaxed);
            }
        }
    }
}

/// Liga a rede: servidor de escuta, ciclo de sincronização e checagem de servidor ao iniciar.
pub fn iniciar(est: Arc<Estado>) {
    if !matches!(est.identidade(), Ok(Some(_))) {
        return;
    }
    // Só inicia se o ciclo estiver parado. Depois de `rede::parar_escuta` (modo local), um novo início
    // reabre a escuta e começa outro ciclo; a thread anterior sai sozinha ao perceber que não é mais a vigente.
    let Some(geracao) = rede::ciclo_iniciar() else { return };
    let para_servidor = est.clone();
    if let Err(e) = rede::ouvir(est.porta, move |s| crate::servidor::tratar_conexao(para_servidor.clone(), s)) {
        eprintln!("não foi possível abrir a porta {}: {e}", est.porta);
    }
    thread::spawn(move || {
        while rede::ciclo_vigente(geracao) {
            thread::sleep(Duration::from_secs(2));
            if !rede::ciclo_vigente(geracao) {
                break;
            }
            ciclo(&est);
        }
    });
}
