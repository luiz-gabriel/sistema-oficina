//! Backup: cópia automática periódica do banco (pasta `backups` dos dados do app) e exportação em JSON
//! para qualquer pasta escolhida (ex.: a pasta do Google Drive sincronizada no computador).
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use oficina_core::error::{AppError, AppResult};
use oficina_core::{config, sync};
use serde_json::{json, Value};

use crate::estado::Estado;
use crate::nuvem;

const INTERVALO: Duration = Duration::from_secs(6 * 60 * 60);
const MANTER: usize = 15;

pub fn pasta(est: &Estado) -> PathBuf {
    est.dir.join("backups")
}

pub fn fazer_backup(est: &Estado) -> AppResult<PathBuf> {
    let dir = pasta(est);
    std::fs::create_dir_all(&dir).map_err(|e| AppError::Database(e.to_string()))?;
    let nome = format!("oficina-{}.db", chrono::Local::now().format("%Y%m%d-%H%M%S"));
    let destino = dir.join(&nome);
    {
        let c = est.db()?;
        c.execute("VACUUM INTO ?1", [destino.to_string_lossy().to_string()])?;
        config::definir(&c, "ultimo_backup", &chrono::Utc::now().to_rfc3339())?;
    }
    limpar_antigos(&dir);
    Ok(destino)
}

fn limpar_antigos(dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    // Só as cópias automáticas ("oficina-*") são apagadas; cópias de segurança de restauração ficam.
    let mut arquivos: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().map_or(false, |x| x == "db") && p.file_name().map_or(false, |n| n.to_string_lossy().starts_with("oficina-")))
        .collect();
    arquivos.sort();
    while arquivos.len() > MANTER {
        let _ = std::fs::remove_file(arquivos.remove(0));
    }
}

pub fn exportar_json(est: &Estado, caminho: &str) -> AppResult<PathBuf> {
    let destino = PathBuf::from(caminho);
    if !destino.is_absolute() {
        return Err(AppError::Conflict("Informe o caminho completo da pasta de destino.".into()));
    }
    let dados = {
        let c = est.db()?;
        // Sem hashes de senha: o arquivo exportado não leva credenciais.
        sync::snapshot_exportacao(&c)?
    };
    let arquivo = json!({
        "formato": "oficina-backup-1",
        "exportado_em": chrono::Utc::now().to_rfc3339(),
        "computador": est.computador,
        "dados": dados,
    });
    let final_ = if destino.is_dir() {
        destino.join(format!("oficina-export-{}.json", chrono::Local::now().format("%Y%m%d-%H%M%S")))
    } else {
        destino
    };
    std::fs::write(&final_, serde_json::to_vec_pretty(&arquivo).map_err(|e| AppError::Database(e.to_string()))?)
        .map_err(|e| AppError::Database(format!("não foi possível gravar: {e}")))?;
    Ok(final_)
}

// ---------- Restauração a partir de um arquivo .db de backup ----------

/// Tabelas criadas automaticamente ao abrir o banco (por versões novas). Um backup antigo que não as tem
/// é aceito: a abertura cria as que faltam. Todas as outras tabelas do esquema são exigidas.
const CRIADAS_AO_ABRIR: &[&str] = &["ordem_km"];

/// Pasta dos arquivos enviados para restauração (fica fora da pasta de cópias automáticas).
fn pasta_restauracao(est: &Estado) -> PathBuf {
    est.dir.join("restauracao")
}

fn tabelas_exigidas() -> Vec<String> {
    oficina_core::db::SCHEMA
        .lines()
        .filter(|l| l.starts_with("CREATE TABLE IF NOT EXISTS "))
        .filter_map(|l| l.split_whitespace().nth(5).map(String::from))
        .filter(|t| !CRIADAS_AO_ABRIR.contains(&t.as_str()))
        .collect()
}

fn conta(conn: &rusqlite::Connection, tabela: &str) -> AppResult<i64> {
    Ok(conn.query_row(&format!("SELECT COUNT(*) FROM {tabela}"), [], |r| r.get(0))?)
}

/// Abre o arquivo só para leitura, confere integridade e tabelas, e devolve um resumo. Não altera o arquivo.
fn conferir(caminho: &Path) -> AppResult<Value> {
    use rusqlite::{Connection, OpenFlags};
    let recusado = || AppError::Conflict("Este arquivo não é um backup compatível com esta versão do sistema.".into());
    let conn = Connection::open_with_flags(caminho, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|_| recusado())?;
    let integro: String = conn.query_row("PRAGMA quick_check", [], |r| r.get(0)).map_err(|_| recusado())?;
    if integro != "ok" {
        return Err(AppError::Conflict("Este arquivo está danificado e não pode ser restaurado.".into()));
    }
    let existentes: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<Result<_, _>>()?;
    let faltam: Vec<String> = tabelas_exigidas().into_iter().filter(|t| !existentes.contains(t)).collect();
    if !faltam.is_empty() {
        return Err(AppError::Conflict(format!(
            "Este backup não é compatível com esta versão do sistema (faltam partes dos dados: {}).",
            faltam.join(", ")
        )));
    }
    let data_arquivo = std::fs::metadata(caminho)
        .and_then(|m| m.modified())
        .ok()
        .map(|t| chrono::DateTime::<chrono::Local>::from(t).format("%d/%m/%Y %H:%M").to_string())
        .unwrap_or_default();
    Ok(json!({
        "data_arquivo": data_arquivo,
        "clientes": conta(&conn, "clientes")?,
        "veiculos": conta(&conn, "veiculos")?,
        "ordens": conta(&conn, "ordens_servico")?,
        "funcionarios": conta(&conn, "funcionarios")?,
    }))
}

/// Recebe o arquivo escolhido (em base64), guarda numa pasta própria e confere se é um backup compatível.
pub fn validar_arquivo(est: &Estado, dados_base64: &str) -> AppResult<Value> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(dados_base64)
        .map_err(|_| AppError::Conflict("Arquivo inválido.".into()))?;
    if !bytes.starts_with(b"SQLite format 3\0") {
        return Err(AppError::Conflict("Este arquivo não é um banco de dados do sistema (.db).".into()));
    }
    let dir = pasta_restauracao(est);
    std::fs::create_dir_all(&dir).map_err(|e| AppError::Database(e.to_string()))?;
    let destino = dir.join(format!("escolhido-{}.db", chrono::Local::now().format("%Y%m%d-%H%M%S%3f")));
    std::fs::write(&destino, &bytes).map_err(|e| AppError::Database(format!("não foi possível guardar o arquivo: {e}")))?;
    match conferir(&destino) {
        Ok(mut resumo) => {
            resumo["caminho"] = json!(destino.to_string_lossy());
            resumo["nome_arquivo"] = json!(destino.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
            Ok(resumo)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&destino);
            Err(e)
        }
    }
}

/// Substitui o banco atual pelo backup escolhido. Antes cria uma cópia de segurança do banco atual.
/// Se a troca falhar, o banco anterior é devolvido a partir dessa cópia.
pub fn restaurar(est: &Estado, caminho: &str) -> AppResult<String> {
    if est.modo() == "cliente" {
        return Err(AppError::Conflict(
            "Neste computador os dados vêm do computador principal. Faça a restauração no computador principal.".into(),
        ));
    }
    // O arquivo precisa estar dentro da pasta de restauração (nada de caminhos arbitrários).
    let dir = pasta_restauracao(est);
    let origem = PathBuf::from(caminho)
        .canonicalize()
        .map_err(|_| AppError::NotFound("Arquivo de backup não encontrado. Escolha o arquivo de novo.".into()))?;
    let dir_canon = dir.canonicalize().map_err(|_| AppError::Conflict("Escolha o arquivo de backup de novo.".into()))?;
    if !origem.starts_with(&dir_canon) {
        return Err(AppError::Conflict("Escolha o arquivo de backup de novo.".into()));
    }
    conferir(&origem)?;

    std::fs::create_dir_all(pasta(est)).map_err(|e| AppError::Database(e.to_string()))?;
    let seguranca = pasta(est).join(format!("antes-restaurar-{}.db", chrono::Local::now().format("%Y%m%d-%H%M%S")));
    {
        let c = est.db()?;
        c.execute("VACUUM INTO ?1", [seguranca.to_string_lossy().to_string()])
            .map_err(|e| AppError::Database(format!("não foi possível criar a cópia de segurança: {e}")))?;
    }

    let banco = est.dir.join("oficina.db");
    {
        // Espera as leituras e escritas em andamento terminarem (cada uma segura a trava) e fecha a piscina antiga.
        let mut trava = est.piscina.write().map_err(|_| AppError::Database("conexão indisponível".into()))?;
        *trava = None;
        let abrir_novo = || -> AppResult<oficina_core::db::Piscina> {
            limpar_wal(&banco);
            std::fs::copy(&origem, &banco).map_err(|e| AppError::Database(format!("não foi possível copiar o arquivo: {e}")))?;
            oficina_core::db::open(&banco)?;
            oficina_core::db::piscina(&banco)
        };
        match abrir_novo() {
            Ok(nova) => *trava = Some(nova),
            Err(erro) => {
                limpar_wal(&banco);
                let voltou = std::fs::copy(&seguranca, &banco).is_ok();
                let reaberta = if voltou {
                    oficina_core::db::open(&banco).ok().and_then(|_| oficina_core::db::piscina(&banco).ok())
                } else {
                    None
                };
                return match reaberta {
                    Some(p) => {
                        *trava = Some(p);
                        Err(AppError::Database(format!("A restauração falhou e os dados anteriores foram mantidos. ({erro})")))
                    }
                    None => Err(AppError::Database(format!(
                        "A restauração falhou e não foi possível reabrir os dados. Reinicie o sistema. Cópia de segurança: {}. ({erro})",
                        seguranca.to_string_lossy()
                    ))),
                };
            }
        }
    }
    // Os dados mudaram: quem estava logado precisa entrar de novo.
    *est.sessao.lock().unwrap() = None;
    let _ = std::fs::remove_file(&origem);
    Ok(seguranca.to_string_lossy().to_string())
}

/// Backup automático a cada 6 horas (e um logo que o app abre, se o último for antigo).
pub fn iniciar_automatico(est: Arc<Estado>) {
    thread::spawn(move || loop {
        let precisa = match est.get("ultimo_backup", "") {
            Ok(v) if !v.is_empty() => chrono::DateTime::parse_from_rfc3339(&v)
                .map(|t| (chrono::Utc::now() - t.with_timezone(&chrono::Utc)).num_seconds() as u64 >= INTERVALO.as_secs())
                .unwrap_or(true),
            _ => true,
        };
        if precisa && est.get("ativado_em", "").map(|v| !v.is_empty()).unwrap_or(false) {
            let _ = fazer_backup(&est);
            nuvem::enviar_automatico(&est);
        }
        thread::sleep(Duration::from_secs(60 * 30));
    });
}

/// Apaga os arquivos auxiliares do modo WAL (`-wal` e `-shm`) de um banco. Sem isso, o SQLite aplicaria
/// ao arquivo restaurado mudanças que pertencem ao banco antigo.
fn limpar_wal(banco: &Path) {
    for sufixo in ["-wal", "-shm"] {
        let mut nome = banco.as_os_str().to_owned();
        nome.push(sufixo);
        let _ = std::fs::remove_file(PathBuf::from(nome));
    }
}

#[cfg(test)]
mod testes {
    use super::*;
    use base64::Engine;

    fn pasta_temporaria(nome: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("oficina-bkp-{nome}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn estado_em(dir: &Path, cliente: &str) -> Estado {
        let est = Estado::novo(dir.to_path_buf(), 7878).unwrap();
        est.db().unwrap().execute("INSERT INTO clientes (nome) VALUES (?1)", [cliente]).unwrap();
        est
    }

    fn nomes_clientes(est: &Estado) -> Vec<String> {
        let c = est.db().unwrap();
        let mut stmt = c.prepare("SELECT nome FROM clientes ORDER BY nome").unwrap();
        let nomes = stmt.query_map([], |r| r.get(0)).unwrap().map(|x| x.unwrap()).collect();
        nomes
    }

    #[test]
    fn tabelas_exigidas_vem_do_esquema_atual_sem_as_criadas_ao_abrir() {
        let t = tabelas_exigidas();
        assert!(t.contains(&"clientes".to_string()));
        assert!(t.contains(&"ordens_servico".to_string()));
        assert!(!t.contains(&"ordem_km".to_string()));
    }

    #[test]
    fn arquivo_que_nao_e_banco_e_recusado() {
        let dir = pasta_temporaria("nao-banco");
        let est = Estado::novo(dir.clone(), 7878).unwrap();
        let b64 = base64::engine::general_purpose::STANDARD.encode(b"texto qualquer");
        assert!(validar_arquivo(&est, &b64).is_err());
    }

    #[test]
    fn banco_de_outra_versao_e_recusado_com_motivo() {
        let dir = pasta_temporaria("incompativel");
        let caminho = dir.join("velho.db");
        rusqlite::Connection::open(&caminho).unwrap().execute_batch("CREATE TABLE clientes (id INTEGER PRIMARY KEY, nome TEXT);").unwrap();
        let est = Estado::novo(dir.clone(), 7878).unwrap();
        let b64 = base64::engine::general_purpose::STANDARD.encode(std::fs::read(&caminho).unwrap());
        let err = validar_arquivo(&est, &b64).unwrap_err();
        assert!(err.to_string().contains("não é compatível"), "{err}");
    }

    #[test]
    fn restaura_troca_os_dados_e_guarda_copia_de_seguranca() {
        let dir = pasta_temporaria("restaura");
        let est = estado_em(&dir, "Cliente atual");
        // Backup feito antes: contém outro cliente.
        let outro = pasta_temporaria("restaura-origem");
        let origem = estado_em(&outro, "Cliente do backup");
        {
            let c = origem.db().unwrap();
            c.execute("VACUUM INTO ?1", [outro.join("copia.db").to_string_lossy().to_string()]).unwrap();
        }
        let b64 = base64::engine::general_purpose::STANDARD.encode(std::fs::read(outro.join("copia.db")).unwrap());
        let resumo = validar_arquivo(&est, &b64).unwrap();
        assert_eq!(resumo["clientes"], 1);
        let caminho = resumo["caminho"].as_str().unwrap().to_string();

        let seguranca = restaurar(&est, &caminho).unwrap();
        assert_eq!(nomes_clientes(&est), vec!["Cliente do backup".to_string()]);
        // A cópia de segurança tem os dados que existiam antes da troca.
        let antigo = rusqlite::Connection::open(&seguranca).unwrap();
        let nome: String = antigo.query_row("SELECT nome FROM clientes", [], |r| r.get(0)).unwrap();
        assert_eq!(nome, "Cliente atual");
        assert!(!Path::new(&caminho).exists(), "arquivo de restauração é apagado depois de usado");
    }

    #[test]
    fn restaurar_recusa_arquivo_fora_da_pasta_de_restauracao() {
        let dir = pasta_temporaria("fora");
        let est = estado_em(&dir, "Atual");
        let err = restaurar(&est, &dir.join("oficina.db").to_string_lossy()).unwrap_err();
        assert!(matches!(err, AppError::Conflict(_)));
        assert_eq!(nomes_clientes(&est), vec!["Atual".to_string()]);
    }
}
