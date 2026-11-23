//! Camada Tauri: expõe ao front-end as operações do sistema (negócio, rede, sessão, backup).
mod backup;
mod estado;
mod nuvem;
mod rede;
mod servico;
mod servidor;
mod sincronia;

use std::sync::Arc;

use estado::Estado;
use oficina_core::error::AppResult;
use oficina_core::rede_regras::modo_usa_rede;
use serde_json::{json, Value};
use tauri::{Manager, State};

type Est<'a> = State<'a, Arc<Estado>>;

/// Liga ou desliga a rede conforme o modo. Só "servidor" e "cliente" abrem escuta e conexões;
/// "local" e modo ainda não escolhido não abrem porta nenhuma.
fn ajustar_rede(est: &Arc<Estado>) {
    if modo_usa_rede(&est.modo()) {
        sincronia::iniciar(est.clone());
    } else {
        rede::parar_escuta(est.porta);
    }
}

#[tauri::command]
fn estado_sistema(est: Est) -> AppResult<Value> {
    servico::estado_sistema(&est)
}

#[tauri::command]
fn ativar_sistema(est: Est, codigo: String) -> AppResult<serde_json::Value> {
    // Ativar não abre rede: o modo ainda não foi escolhido.
    servico::ativar(&est, &codigo)
}

#[tauri::command]
fn criar_administrador(est: Est, nome: String, login: String, senha: String) -> AppResult<()> {
    servico::criar_administrador(&est, &nome, &login, &senha)
}

#[tauri::command]
fn login(est: Est, login: String, senha: String) -> AppResult<Value> {
    servico::login(&est, &login, &senha)
}

#[tauri::command]
fn logout(est: Est) {
    servico::logout(&est)
}

#[tauri::command]
fn comando(est: Est, nome: String, args: Value) -> AppResult<Value> {
    servico::comando(&est, &nome, &args)
}

#[tauri::command]
fn config_obter(est: Est) -> AppResult<Value> {
    servico::config_obter(&est)
}

#[tauri::command]
async fn config_modo(est: Est<'_>, modo: String, endereco: Option<String>) -> AppResult<()> {
    servico::config_modo(&est, &modo, endereco)?;
    ajustar_rede(est.inner());
    Ok(())
}

#[tauri::command]
async fn rede_buscar(est: Est<'_>) -> AppResult<Value> {
    // Varrer a sub-rede é conexão de rede: fora dos modos de rede, não faz nada.
    if !modo_usa_rede(&est.modo()) {
        return Ok(json!([]));
    }
    servico::rede_buscar(&est)
}

#[tauri::command]
async fn sincronizar_agora(est: Est<'_>) -> AppResult<Value> {
    servico::sincronizar_agora(&est)
}

#[tauri::command]
async fn trabalhar_offline(est: Est<'_>, ligar: bool) -> AppResult<()> {
    servico::trabalhar_offline(&est, ligar)
}

#[tauri::command]
fn pendencias(est: Est) -> AppResult<Value> {
    servico::pendencias(&est)
}

#[tauri::command]
fn historico(est: Est, limite: i64) -> AppResult<Value> {
    servico::historico(&est, limite)
}

#[tauri::command]
fn versoes_conflito(est: Est, uuid: String) -> AppResult<Value> {
    servico::versoes_conflito(&est, &uuid)
}

#[tauri::command]
async fn resolver_conflito(est: Est<'_>, uuid: String, escolha: String) -> AppResult<()> {
    servico::resolver_conflito(&est, &uuid, &escolha)
}

#[tauri::command]
async fn impressora_atual(est: Est<'_>) -> AppResult<String> {
    servico::impressora_atual(&est)
}

#[tauri::command]
async fn impressora_salvar(est: Est<'_>, destino: String) -> AppResult<()> {
    servico::impressora_salvar(&est, &destino)
}

#[tauri::command]
async fn impressora_testar(est: Est<'_>) -> AppResult<()> {
    servico::impressora_testar(&est)
}

#[tauri::command]
async fn ordens_imprimir(est: Est<'_>, id: i64) -> AppResult<()> {
    servico::imprimir_os(&est, id)
}

#[tauri::command]
async fn orcamento_imprimir(est: Est<'_>, id: i64) -> AppResult<()> {
    servico::imprimir_orcamento(&est, id)
}

/// Imprime o fechamento de um caixa. Sem impressora configurada, devolve aviso claro e não mexe no caixa.
#[tauri::command]
async fn caixa_imprimir(est: Est<'_>, id: i64) -> AppResult<()> {
    use oficina_core::{caixa, impressao, AppError};
    let relatorio = {
        let c = est.db()?;
        caixa::relatorio(&c, id)?
    };
    let texto = servico::impressora_atual(&est)?;
    if texto.trim().is_empty() {
        return Err(AppError::Conflict(
            "Nenhuma impressora configurada. Escolha em Configurações > Impressora térmica.".into(),
        ));
    }
    let dest = impressao::destino(&texto)?;
    impressao::enviar(&dest, &impressao::cupom_fechamento(&relatorio)?)
}

#[tauri::command]
fn registro_listar(est: Est, limite: i64) -> AppResult<Value> {
    servico::registro_listar(&est, limite)
}

#[tauri::command]
fn logo_obter(est: Est) -> AppResult<String> {
    servico::logo_obter(&est)
}

#[tauri::command]
fn logo_salvar(est: Est, dados: String) -> AppResult<()> {
    servico::logo_salvar(&est, &dados)
}

#[tauri::command]
fn backup_agora(est: Est) -> AppResult<String> {
    servico::exige_administrador(&est)?;
    Ok(backup::fazer_backup(&est)?.to_string_lossy().to_string())
}

#[tauri::command]
fn exportar_dados(est: Est, caminho: String) -> AppResult<String> {
    servico::exige_administrador(&est)?;
    Ok(backup::exportar_json(&est, &caminho)?.to_string_lossy().to_string())
}

/// Recebe o arquivo .db escolhido (base64) e devolve o resumo para confirmação. Não troca nada ainda.
#[tauri::command]
fn backup_validar_arquivo(est: Est, dados: String) -> AppResult<Value> {
    servico::exige_administrador(&est)?;
    backup::validar_arquivo(&est, &dados)
}

/// Restaura o backup validado. Exige a palavra RESTAURAR digitada pelo administrador.
#[tauri::command]
fn backup_restaurar(est: Est, caminho: String, confirmacao: String) -> AppResult<String> {
    servico::exige_administrador(&est)?;
    if confirmacao.trim() != "RESTAURAR" {
        return Err(oficina_core::AppError::Conflict("Para confirmar, digite a palavra RESTAURAR.".into()));
    }
    backup::restaurar(&est, &caminho)
}

#[tauri::command]
fn nuvem_configurar(est: Est, client_id: String, client_secret: String, manter: i64) -> AppResult<()> {
    nuvem::configurar(&est, &client_id, &client_secret, manter)
}

#[tauri::command]
fn nuvem_conectar(est: Est) -> AppResult<Value> {
    nuvem::conectar(est.inner())
}

#[tauri::command]
fn nuvem_status(est: Est) -> AppResult<Value> {
    nuvem::status(&est)
}

#[tauri::command]
async fn nuvem_enviar(est: Est<'_>) -> AppResult<Value> {
    let e = est.inner().clone();
    nuvem::em_segundo_plano(move || nuvem::enviar_agora(&e)).await
}

#[tauri::command]
async fn nuvem_listar(est: Est<'_>) -> AppResult<Value> {
    let e = est.inner().clone();
    nuvem::em_segundo_plano(move || nuvem::listar_agora(&e)).await
}

#[tauri::command]
async fn nuvem_baixar(est: Est<'_>, id: String) -> AppResult<String> {
    let e = est.inner().clone();
    nuvem::em_segundo_plano(move || nuvem::baixar_agora(&e, &id)).await
}

#[tauri::command]
async fn nuvem_desconectar(est: Est<'_>) -> AppResult<()> {
    let e = est.inner().clone();
    nuvem::em_segundo_plano(move || nuvem::desconectar_agora(&e)).await
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            let conn = oficina_core::db::open(&dir.join("oficina.db"))
                .map_err(|e| format!("não foi possível abrir o banco: {e}"))?;
            let porta = std::env::var("OFICINA_PORTA").ok().and_then(|p| p.parse().ok()).unwrap_or(7878);
            let est = Arc::new(Estado::novo(conn, dir, porta));
            ajustar_rede(&est);
            backup::iniciar_automatico(est.clone());
            app.manage(est);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            estado_sistema, ativar_sistema, criar_administrador, login, logout, comando,
            config_obter, config_modo, rede_buscar, sincronizar_agora, trabalhar_offline,
            pendencias, historico, resolver_conflito, versoes_conflito, backup_agora, exportar_dados,
            backup_validar_arquivo, backup_restaurar,
            impressora_atual, impressora_salvar, impressora_testar, ordens_imprimir, orcamento_imprimir, caixa_imprimir,
            registro_listar, logo_obter, logo_salvar,
            nuvem_configurar, nuvem_conectar, nuvem_status, nuvem_enviar, nuvem_listar, nuvem_baixar, nuvem_desconectar,
        ])
        .run(tauri::generate_context!())
        .expect("erro ao iniciar o aplicativo");
}
