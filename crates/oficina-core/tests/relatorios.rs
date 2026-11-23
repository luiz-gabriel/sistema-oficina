// Relatórios de gestão com um cenário pequeno. Os valores esperados foram conferidos à mão (centavos).
//
// Cenário (período 01/11/2026 a 30/11/2026, hoje = 22/11/2026):
//  OS 1  (Carlos)   concluída 10/11, recebida no pix,    filtro 30 + troca de óleo 50   = 80
//  OS 2  (Mariana)  concluída 12/11, recebida no cartão, 2 pastilhas 160 + alinhamento 70 = 230
//  Venda 3 (Carlos) concluída 12/11, recebida em dinheiro, filtro 30 (balcão)            = 30
//  OS 4  (Carlos)   concluída 05/10, fora do período                                     = 999
//  OS 5  (Mariana)  criada 15/11, em aberto, prazo previsto 20/11 (já vencido)          = 50
//  Compra de 4 filtros em 05/11 por 48,00 (12,00 cada) -> custo médio do filtro = 12,00.
//  Despesa: aluguel 500,00 lançado em 02/11 (sem compra). Conta da compra (200,00) paga em 06/11.
//  Contas a receber: OS 5, saldo 50,00, vence 20/11 (vencida).
//  Contas a pagar: aluguel 500,00 sem pagamento, vence 05/11 (vencida).
use chrono::NaiveDate;
use oficina_core::db;
use oficina_core::relatorios::{self, Periodo, Total};
use rusqlite::Connection;

fn cenario() -> Connection {
    let conn = db::open_in_memory().unwrap();
    conn.execute_batch(
        "INSERT INTO clientes (id, nome) VALUES (1, 'Ana');
         INSERT INTO veiculos (id, cliente_id, placa, modelo) VALUES (1, 1, 'ABC1D23', 'Polo');
         INSERT INTO funcionarios (id, nome) VALUES (1, 'Carlos'), (2, 'Mariana');
         INSERT INTO itens (id, nome, tipo, preco_custo, preco_venda, quantidade, estoque_minimo) VALUES
           (1, 'Filtro de óleo', 'peca', 10, 30, 5, 2),
           (2, 'Pastilha', 'peca', 40, 80, 1, 2),
           (3, 'Troca de óleo', 'servico', 0, 50, 0, 0),
           (4, 'Alinhamento', 'servico', 0, 70, 0, 0);
         INSERT INTO ordens_servico (id, tipo, veiculo_id, cliente_id, funcionario_id, status, data_entrada, data_conclusao, valor_total, pagamento) VALUES
           (1, 'OS', 1, NULL, 1, 'CONCLUIDA', '2026-11-10T09:00:00+00:00', '2026-11-10T12:00:00+00:00', 80, 'pix'),
           (2, 'OS', 1, NULL, 2, 'CONCLUIDA', '2026-11-11T09:00:00+00:00', '2026-11-12T12:00:00+00:00', 230, 'cartao'),
           (3, 'BALCAO', NULL, 1, 1, 'CONCLUIDA', '2026-11-12T15:00:00+00:00', '2026-11-12T15:00:00+00:00', 30, 'dinheiro'),
           (4, 'OS', 1, NULL, 1, 'CONCLUIDA', '2026-10-05T09:00:00+00:00', '2026-10-05T12:00:00+00:00', 999, 'pix'),
           (5, 'OS', 1, NULL, 2, 'CRIADA', '2026-11-15T09:00:00+00:00', NULL, 50, NULL);
         INSERT INTO ordem_itens (os_id, item_id, quantidade, preco_unitario, subtotal) VALUES
           (1, 1, 1, 30, 30), (1, 3, 1, 50, 50),
           (2, 2, 2, 80, 160), (2, 4, 1, 70, 70),
           (3, 1, 1, 30, 30),
           (4, 1, 1, 30, 30),
           (5, 3, 1, 50, 50);
         INSERT INTO ordem_km (os_id, prazo_previsto) VALUES (5, '2026-11-20');
         INSERT INTO recebimentos (ordem_id, valor_centavos, forma, data, usuario) VALUES
           (1, 8000, 'pix', '2026-11-10T09:00:00+00:00', 'Carlos'),
           (2, 23000, 'cartao', '2026-11-12T09:00:00-03:00', 'Mariana'),
           (3, 3000, 'dinheiro', '2026-11-12T15:00:00+00:00', 'Carlos'),
           (4, 99900, 'pix', '2026-10-05T12:00:00+00:00', 'Carlos');
         INSERT INTO contas_receber (id, origem, referencia, cliente_id, descricao, valor_total_centavos, saldo_centavos, vencimento, data)
           VALUES (1, 'OS', 5, 1, 'OS 5', 5000, 5000, '2026-11-20', '2026-11-15T09:00:00+00:00');
         INSERT INTO fornecedores (id, nome, documento) VALUES (1, 'Peças SA', '00000000000191');
         INSERT INTO compras (id, fornecedor_id, data, total_centavos, pagamento, paga, criado_em)
           VALUES (1, 1, '2026-11-05', 4800, 'pix', 1, '2026-11-05T10:00:00+00:00');
         INSERT INTO compra_itens (compra_id, item_id, quantidade, custo_unitario_centavos, subtotal_centavos)
           VALUES (1, 1, 4, 1200, 4800);
         INSERT INTO contas_pagar (id, descricao, compra_id, valor_centavos, vencimento, data, usuario) VALUES
           (1, 'Aluguel', NULL, 50000, '2026-11-05', '2026-11-02T10:00:00+00:00', 'Carlos'),
           (2, 'Compra de filtros', 1, 20000, '2026-11-05', '2026-11-05T10:00:00+00:00', 'Carlos'),
           (3, 'Luz de outubro', NULL, 7000, '2026-10-10', '2026-10-10T10:00:00+00:00', 'Carlos');
         INSERT INTO pagamentos_pagar (conta_id, valor_centavos, data, usuario) VALUES
           (2, 20000, '2026-11-06T10:00:00+00:00', 'Carlos');",
    )
    .unwrap();
    conn
}

fn novembro() -> Periodo {
    Periodo::novo("2026-11-01", "2026-11-30").unwrap()
}

fn hoje() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, 22).unwrap()
}

#[test]
fn faturamento_separa_os_e_vendas_do_periodo() {
    let conn = cenario();
    let f = relatorios::faturamento(&conn, &novembro()).unwrap();
    assert_eq!(f.servicos_os, Total { quantidade: 2, centavos: 31_000 });
    assert_eq!(f.vendas_balcao, Total { quantidade: 1, centavos: 3_000 });
    assert_eq!(f.total, Total { quantidade: 3, centavos: 34_000 });
}

#[test]
fn recebimentos_por_forma_vem_da_tabela_de_recebimentos() {
    let conn = cenario();
    let r = relatorios::recebimentos(&conn, &novembro()).unwrap();
    assert_eq!(r.dinheiro, Total { quantidade: 1, centavos: 3_000 });
    assert_eq!(r.pix, Total { quantidade: 1, centavos: 8_000 });
    // 12/11 às 09h de Brasília = 12h UTC: dentro do período, mesmo com outro fuso gravado.
    assert_eq!(r.cartao, Total { quantidade: 1, centavos: 23_000 });
    assert_eq!(r.outro, Total::default());
    assert_eq!(r.total, Total { quantidade: 3, centavos: 34_000 });
}

#[test]
fn contas_mostram_saldos_e_vencidos() {
    let conn = cenario();
    let c = relatorios::contas(&conn, hoje()).unwrap();
    assert_eq!(c.ordens_em_aberto, Total { quantidade: 1, centavos: 5_000 });
    assert_eq!(c.ordens_com_prazo_vencido, Total { quantidade: 1, centavos: 5_000 });
    assert_eq!(c.contas_a_receber, Total { quantidade: 1, centavos: 5_000 });
    assert_eq!(c.contas_a_receber_vencidas, Total { quantidade: 1, centavos: 5_000 });
    // Aluguel (500,00) e luz de outubro (70,00) ainda não pagos; a compra já foi paga.
    assert_eq!(c.contas_a_pagar, Total { quantidade: 2, centavos: 57_000 });
    assert_eq!(c.contas_a_pagar_vencidas, Total { quantidade: 2, centavos: 57_000 });
}

#[test]
fn lucro_usa_custo_medio_da_compra_e_despesas_do_periodo() {
    let conn = cenario();
    let l = relatorios::lucro(&conn, &novembro()).unwrap();
    assert_eq!(l.receita_centavos, 34_000);
    // Filtro: custo médio 12,00 (compra). Filtro 1 (OS 1) + filtro 1 (venda 3) = 2 × 12 = 24,00.
    // Pastilha sem compra: custo do cadastro 40,00 × 2 = 80,00. Total 104,00.
    assert_eq!(l.custo_pecas_centavos, 10_400);
    // Só o aluguel: a compra de peças não é despesa, e a luz é de outubro.
    assert_eq!(l.despesas_centavos, 50_000);
    assert_eq!(l.resultado_centavos, 34_000 - 10_400 - 50_000);
    assert!(l.nao_incluido.iter().any(|t| t.starts_with("Mão de obra")));
    assert!(l.nao_incluido.iter().any(|t| t.starts_with("Compras de peças")));
}

#[test]
fn servicos_somam_todos_e_listam_os_mais_realizados() {
    let conn = cenario();
    let s = relatorios::servicos(&conn, &novembro()).unwrap();
    assert_eq!(s.quantidade, 2);
    assert_eq!(s.centavos, 12_000);
    let nomes: Vec<&str> = s.mais_realizados.iter().map(|x| x.nome.as_str()).collect();
    // Mesma quantidade (1): o de maior valor vem primeiro.
    assert_eq!(nomes, vec!["Alinhamento", "Troca de óleo"]);
    assert_eq!(s.mais_realizados[0].centavos, 7_000);
}

#[test]
fn produtividade_usa_o_responsavel_pela_os() {
    let conn = cenario();
    let p = relatorios::produtividade(&conn, &novembro()).unwrap();
    assert_eq!(p.len(), 2);
    // Ordenado pelo valor: Mariana (230) antes de Carlos (80 + 30).
    assert_eq!(p[0].responsavel_nome, "Mariana");
    assert_eq!(p[0].ordens_concluidas, 1);
    assert_eq!(p[0].centavos, 23_000);
    assert_eq!(p[0].servicos_lancados, 1);
    assert_eq!(p[1].responsavel_nome, "Carlos");
    assert_eq!(p[1].ordens_concluidas, 2);
    assert_eq!(p[1].centavos, 11_000);
    assert_eq!(p[1].servicos_lancados, 1);
}

#[test]
fn estoque_mostra_saldo_baixo_saidas_e_entradas() {
    let conn = cenario();
    let e = relatorios::estoque(&conn, &novembro()).unwrap();
    assert_eq!(e.saldo_baixo.len(), 1);
    assert_eq!(e.saldo_baixo[0].nome, "Pastilha");
    assert_eq!(e.saidas_quantidade, 4);
    assert_eq!(e.saidas_centavos, 22_000);
    let filtro = e.saidas.iter().find(|s| s.nome == "Filtro de óleo").unwrap();
    assert_eq!((filtro.quantidade, filtro.centavos), (2, 6_000));
    let pastilha = e.saidas.iter().find(|s| s.nome == "Pastilha").unwrap();
    assert_eq!((pastilha.quantidade, pastilha.centavos), (2, 16_000));
    // Entrada: compra de 4 filtros em 05/11 por 48,00.
    assert_eq!((e.entradas_quantidade, e.entradas_centavos), (4, 4_800));
}

#[test]
fn ordem_fora_do_periodo_nao_entra_em_nada() {
    let conn = cenario();
    let outubro = Periodo::novo("2026-10-01", "2026-10-31").unwrap();
    let f = relatorios::faturamento(&conn, &outubro).unwrap();
    assert_eq!(f.total, Total { quantidade: 1, centavos: 99_900 });
    let r = relatorios::recebimentos(&conn, &outubro).unwrap();
    assert_eq!(r.total, Total { quantidade: 1, centavos: 99_900 });
    let l = relatorios::lucro(&conn, &outubro).unwrap();
    // Outubro: OS 4 (filtro com custo médio 12,00) e a luz de 70,00.
    assert_eq!(l.receita_centavos, 99_900);
    assert_eq!(l.custo_pecas_centavos, 1_200);
    assert_eq!(l.despesas_centavos, 7_000);
}

#[test]
fn ultimo_dia_entra_inteiro() {
    let conn = cenario();
    // Venda de 12/11 às 15h UTC: um período que termina em 12/11 precisa conter o dia inteiro.
    let p = Periodo::novo("2026-11-12", "2026-11-12").unwrap();
    let f = relatorios::faturamento(&conn, &p).unwrap();
    assert_eq!(f.total, Total { quantidade: 2, centavos: 26_000 });
}

#[test]
fn periodo_invalido_e_recusado() {
    assert!(Periodo::novo("2026-11-30", "2026-11-01").is_err());
    assert!(Periodo::novo("31/11/2026", "2026-12-01").is_err());
    assert!(Periodo::novo("2020-01-01", "2026-12-01").is_err());
}

#[test]
fn relatorio_completo_reune_todas_as_secoes() {
    let conn = cenario();
    let r = relatorios::completo(&conn, &novembro(), hoje()).unwrap();
    assert_eq!(r.de, "2026-11-01");
    assert_eq!(r.faturamento.total.centavos, 34_000);
    assert_eq!(r.recebimentos.total.centavos, 34_000);
    assert_eq!(r.produtividade.len(), 2);
    assert_eq!(r.estoque.saldo_baixo.len(), 1);
}
