//! Teste de estresse (fora da suíte normal, por `#[ignore]`). Usa o mesmo caminho dos comandos do app
//! (`comandos::executar`) e um banco em arquivo, com fsync real.
//!
//! Rodar:
//!   cargo test --release -p oficina-core --test estresse -- --ignored --nocapture
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use oficina_core::caixa;
use oficina_core::comandos::{executar, Contexto};
use oficina_core::db;
use rusqlite::Connection;
use serde_json::{json, Value};

const CLIENTES: usize = 4500;
const VEICULOS: usize = 9000;
const PECAS: usize = 4000;
const VENDAS: usize = 4000;
const THREADS: usize = 4;
const OPS_POR_THREAD: usize = 250;

fn ctx() -> Contexto {
    Contexto { usuario: "Carlos".into(), perfil: "administrador".into(), computador: "PC1".into() }
}

#[derive(Default)]
struct Fase {
    nome: &'static str,
    ok: usize,
    erros: Vec<String>,
    tempos: Vec<Duration>,
    total: Duration,
}

impl Fase {
    fn registrar(&mut self, inicio: Instant, r: Result<Value, oficina_core::error::AppError>) -> Option<Value> {
        let d = inicio.elapsed();
        self.tempos.push(d);
        match r {
            Ok(v) => {
                self.ok += 1;
                Some(v)
            }
            Err(e) => {
                if self.erros.len() < 5 {
                    self.erros.push(format!("{e:?}"));
                }
                None
            }
        }
    }

    fn imprimir(&mut self) {
        self.tempos.sort();
        let p = |q: f64| -> f64 {
            if self.tempos.is_empty() {
                return 0.0;
            }
            let i = ((self.tempos.len() as f64 - 1.0) * q).round() as usize;
            self.tempos[i].as_secs_f64() * 1000.0
        };
        let total = self.tempos.iter().sum::<Duration>().as_secs_f64();
        let media = if self.tempos.is_empty() { 0.0 } else { total * 1000.0 / self.tempos.len() as f64 };
        println!(
            "[{}] ok={} erros={} | {:.1} ops/s | média {:.2} ms | p50 {:.2} | p95 {:.2} | p99 {:.2} | máx {:.2} ms",
            self.nome,
            self.ok,
            self.tempos.len() - self.ok,
            self.ok as f64 / total.max(1e-9),
            media,
            p(0.50),
            p(0.95),
            p(0.99),
            p(1.0),
        );
        for e in &self.erros {
            println!("    exemplo de erro: {e}");
        }
    }
}

fn placa(i: usize) -> String {
    format!("ABC{:04}", i)
}

#[test]
#[ignore]
fn estresse_volume_sequencial() {
    let dir = std::env::temp_dir().join(format!("oficina_estresse_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let caminho = dir.join("oficina.db");
    let conn = db::open(&caminho).unwrap();
    conn.execute("INSERT INTO funcionarios (id, nome) VALUES (1, 'Carlos')", []).unwrap();
    let ctx = ctx();

    // 1) Clientes.
    let mut cliente_ids = Vec::with_capacity(CLIENTES);
    let mut fase = Fase { nome: "clientes_criar", ..Default::default() };
    let t0 = Instant::now();
    for i in 0..CLIENTES {
        let dados = json!({"dados": {"nome": format!("Cliente {i:05}"), "telefone": format!("1199{i:07}")}});
        let inicio = Instant::now();
        if let Some(v) = fase.registrar(inicio, executar(&conn, "clientes_criar", &dados, &ctx)) {
            cliente_ids.push(v["id"].as_i64().unwrap());
        }
    }
    fase.total = t0.elapsed();
    fase.imprimir();

    // 2) Veículos: o dobro dos clientes, distribuídos entre eles.
    let mut veiculo_ids = Vec::with_capacity(VEICULOS);
    let mut fase = Fase { nome: "veiculos_criar", ..Default::default() };
    for i in 0..VEICULOS {
        let dados = json!({"dados": {
            "cliente_id": cliente_ids[i % cliente_ids.len()],
            "placa": placa(i),
            "marca": "VW",
            "modelo": "Polo",
            "ano": 2015,
        }});
        let inicio = Instant::now();
        if let Some(v) = fase.registrar(inicio, executar(&conn, "veiculos_criar", &dados, &ctx)) {
            veiculo_ids.push(v["id"].as_i64().unwrap());
        }
    }
    fase.imprimir();

    // 3) Peças no estoque (quantidade 50 cada).
    let mut peca_ids = Vec::with_capacity(PECAS);
    let mut fase = Fase { nome: "itens_criar (peças)", ..Default::default() };
    for i in 0..PECAS {
        let dados = json!({"dados": {
            "nome": format!("Peça {i:05}"),
            "tipo": "peca",
            "codigo": format!("7890{i:09}"),
            "preco_custo": 10.0,
            "preco_venda": 25.0,
            "quantidade": 50,
            "estoque_minimo": 5,
        }});
        let inicio = Instant::now();
        if let Some(v) = fase.registrar(inicio, executar(&conn, "itens_criar", &dados, &ctx)) {
            peca_ids.push(v["id"].as_i64().unwrap());
        }
    }
    fase.imprimir();
    let estoque_inicial: i64 = conn.query_row("SELECT SUM(quantidade) FROM itens WHERE tipo = 'peca'", [], |r| r.get(0)).unwrap();

    // 4) Vendas de balcão: uma peça por venda, com caixa aberto.
    caixa::abrir(&conn, Some(100.0), "Carlos", &chrono::Utc::now().to_rfc3339()).unwrap();
    let mut fase = Fase { nome: "venda_balcao_criar", ..Default::default() };
    for i in 0..VENDAS {
        let dados = json!({"dados": {
            "cliente_id": cliente_ids[i % cliente_ids.len()],
            "funcionario_id": 1,
            "pagamento": "pix",
            "itens": [{"item_id": peca_ids[i % peca_ids.len()], "quantidade": 1}],
        }});
        let inicio = Instant::now();
        fase.registrar(inicio, executar(&conn, "venda_balcao_criar", &dados, &ctx));
    }
    fase.imprimir();

    // 5) Telas de listagem com o banco cheio (as listas não têm paginação).
    println!("== Listagens com o banco cheio");
    let listagens: [(&str, Value); 9] = [
        ("clientes_listar", json!({})),
        ("clientes_pagina", json!({"limite": 80, "deslocamento": 0})),
        ("veiculos_listar", json!({})),
        ("veiculos_pagina", json!({"limite": 80, "deslocamento": 0})),
        ("itens_listar", json!({})),
        ("itens_pagina", json!({"limite": 80, "deslocamento": 0})),
        ("ordens_listar", json!({})),
        ("ordens_pagina", json!({"limite": 80, "deslocamento": 0})),
        ("dashboard_resumo", json!({})),
    ];
    for (cmd, args) in listagens {
        let mut tempos = Vec::new();
        let mut tamanho = 0usize;
        for _ in 0..5 {
            let t = Instant::now();
            let v = executar(&conn, cmd, &args, &ctx).unwrap();
            tempos.push(t.elapsed());
            tamanho = v.to_string().len();
        }
        tempos.sort();
        println!("{cmd:<18} mediana {:>8.1} ms | resposta JSON {:>8.1} KB", tempos[2].as_secs_f64() * 1000.0, tamanho as f64 / 1024.0);
    }

    // 6) Consistência depois da carga.
    println!("== Consistência");
    let integ: String = conn.query_row("PRAGMA integrity_check", [], |r| r.get(0)).unwrap();
    println!("integrity_check: {integ}");
    let fk: i64 = conn.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| r.get(0)).unwrap_or(-1);
    println!("violações de chave estrangeira: {fk}");
    let estoque_final: i64 = conn.query_row("SELECT SUM(quantidade) FROM itens WHERE tipo = 'peca'", [], |r| r.get(0)).unwrap();
    let vendas_ok: i64 = conn
        .query_row("SELECT COUNT(*) FROM ordens_servico WHERE tipo = 'BALCAO' AND status = 'CONCLUIDA'", [], |r| r.get(0))
        .unwrap();
    println!("estoque: {estoque_inicial} -> {estoque_final} (vendidas: {vendas_ok}, baixa esperada: {})", estoque_inicial - estoque_final);
    assert_eq!(estoque_inicial - estoque_final, vendas_ok, "a baixa de estoque tem que bater com as vendas");
    let tamanho = std::fs::metadata(&caminho).map(|m| m.len()).unwrap_or(0);
    println!("tamanho do banco: {:.1} MB", tamanho as f64 / 1_048_576.0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore]
fn estresse_concorrente_com_mutex_como_no_app() {
    // O app guarda uma única conexão atrás de um Mutex (ver `Estado::db`). Aqui várias threads disputam esse mutex.
    let dir = std::env::temp_dir().join(format!("oficina_estresse_conc_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let conn = db::open(&dir.join("oficina.db")).unwrap();
    conn.execute_batch(
        "INSERT INTO funcionarios (id, nome) VALUES (1, 'Carlos');
         INSERT INTO clientes (id, nome) VALUES (1, 'Base');",
    )
    .unwrap();
    for i in 1..=THREADS * OPS_POR_THREAD {
        conn.execute(
            "INSERT INTO itens (id, nome, tipo, codigo, preco_venda, quantidade, estoque_minimo) VALUES (?1, ?2, 'peca', ?3, 25, 1000, 5)",
            rusqlite::params![i as i64, format!("P{i}"), format!("CC{i:08}")],
        )
        .unwrap();
    }
    caixa::abrir(&conn, Some(0.0), "Carlos", &chrono::Utc::now().to_rfc3339()).unwrap();
    let conn: Arc<Mutex<Connection>> = Arc::new(Mutex::new(conn));

    let inicio = Instant::now();
    let handles: Vec<_> = (0..THREADS)
        .map(|t| {
            let conn = conn.clone();
            std::thread::spawn(move || {
                let ctx = ctx();
                let mut tempos = Vec::new();
                let mut ok = 0usize;
                let mut erros = Vec::new();
                for k in 0..OPS_POR_THREAD {
                    let peca = (t * OPS_POR_THREAD + k + 1) as i64;
                    let dados = json!({"dados": {"cliente_id": 1, "funcionario_id": 1, "pagamento": "pix",
                        "itens": [{"item_id": peca, "quantidade": 1}]}});
                    let t1 = Instant::now();
                    let r = {
                        let c = conn.lock().unwrap();
                        executar(&c, "venda_balcao_criar", &dados, &ctx)
                    };
                    tempos.push(t1.elapsed());
                    match r {
                        Ok(_) => ok += 1,
                        Err(e) => erros.push(format!("{e:?}")),
                    }
                }
                (tempos, ok, erros)
            })
        })
        .collect();
    let mut tempos = Vec::new();
    let mut ok = 0;
    let mut erros = Vec::new();
    for h in handles {
        let (t, o, e) = h.join().unwrap();
        tempos.extend(t);
        ok += o;
        erros.extend(e);
    }
    let total = inicio.elapsed();
    tempos.sort();
    let p = |q: f64| tempos[((tempos.len() as f64 - 1.0) * q).round() as usize].as_secs_f64() * 1000.0;
    println!(
        "[concorrente {THREADS} threads] ok={ok} erros={} | {:.1} vendas/s | p50 {:.2} ms | p95 {:.2} ms | p99 {:.2} ms | máx {:.2} ms | tempo total {:.2} s",
        erros.len(),
        ok as f64 / total.as_secs_f64(),
        p(0.5),
        p(0.95),
        p(0.99),
        p(1.0),
        total.as_secs_f64()
    );
    for e in erros.iter().take(5) {
        println!("    exemplo de erro: {e}");
    }
    let guard = conn.lock().unwrap();
    let vendas: i64 = guard
        .query_row("SELECT COUNT(*) FROM ordens_servico WHERE tipo = 'BALCAO' AND status = 'CONCLUIDA'", [], |r| r.get(0))
        .unwrap();
    println!("vendas gravadas: {vendas} (esperado {})", ok);
    assert_eq!(vendas as usize, ok);
    drop(guard);
    let _ = std::fs::remove_dir_all(&dir);
}
