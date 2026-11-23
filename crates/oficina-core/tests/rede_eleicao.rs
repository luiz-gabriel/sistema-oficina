// Regra de escolha do servidor (função pura): época, antiguidade, quórum e uma única promoção.
use oficina_core::rede_regras::{decidir_promocao, decidir_servidor, modo_usa_rede, proxima_epoca, servidor_aceitavel, Decisao, Membro};

fn m(dispositivo: &str, ativado_em: &str, papel: &str) -> Membro {
    Membro { dispositivo: dispositivo.into(), ativado_em: ativado_em.into(), papel: papel.into(), epoca: 0 }
}

fn com_epoca(dispositivo: &str, ativado_em: &str, papel: &str, epoca: u64) -> Membro {
    Membro { epoca, ..m(dispositivo, ativado_em, papel) }
}

const JAN: &str = "2024-01-10T09:00:00+00:00";
const FEV: &str = "2024-02-10T09:00:00+00:00";
const MAR: &str = "2024-03-10T09:00:00+00:00";

#[test]
fn so_o_mais_antigo_com_quorum_se_promove() {
    let a = m("dev-a", JAN, "cliente");
    let b = m("dev-b", FEV, "cliente");
    let c = m("dev-c", MAR, "cliente");
    assert_eq!(decidir_promocao(&a, &[b.clone(), c.clone()], 3), Decisao::Promover);
    assert!(matches!(decidir_promocao(&b, &[a.clone(), c.clone()], 3), Decisao::Aguardar(_)));
    assert!(matches!(decidir_promocao(&c, &[a, b], 3), Decisao::Aguardar(_)));
}

#[test]
fn sem_quorum_ninguem_se_promove() {
    // Conhecidos: 4. Este computador vê só mais um (2 de 4 não é maioria estrita).
    let eu = m("dev-a", JAN, "sem_servidor");
    let outro = m("dev-b", FEV, "sem_servidor");
    assert!(matches!(decidir_promocao(&eu, &[outro], 4), Decisao::Aguardar(_)));
}

#[test]
fn divisao_da_rede_gera_no_maximo_um_servidor() {
    // 5 conhecidos divididos em {A1, A2} e {B1, B2, B3}. A1 é o mais antigo de A, B1 o mais antigo de B.
    let a1 = m("dev-a1", JAN, "cliente");
    let a2 = m("dev-a2", FEV, "cliente");
    let b1 = m("dev-b1", JAN, "cliente");
    let b2 = m("dev-b2", FEV, "cliente");
    let b3 = m("dev-b3", MAR, "cliente");
    // Grupo A: 2 de 5, sem maioria.
    assert!(matches!(decidir_promocao(&a1, &[a2], 5), Decisao::Aguardar(_)));
    // Grupo B: 3 de 5, maioria. Só o mais antigo de B se promove.
    assert_eq!(decidir_promocao(&b1, &[b2.clone(), b3.clone()], 5), Decisao::Promover);
    assert!(matches!(decidir_promocao(&b2, &[b1, b3], 5), Decisao::Aguardar(_)));
}

#[test]
fn servidor_respondendo_bloqueia_a_promocao() {
    let eu = m("dev-a", JAN, "cliente");
    let servidor = m("dev-s", "2023-01-01T00:00:00+00:00", "servidor");
    assert!(matches!(decidir_promocao(&eu, &[servidor], 2), Decisao::Aguardar(_)));
}

#[test]
fn data_de_ativacao_vence_antes_do_identificador() {
    // "dev-a" < "dev-z" em texto, mas a instalação de "dev-z" é mais antiga.
    let eu = m("dev-a", "2024-05-01T00:00:00+00:00", "cliente");
    let outro = m("dev-z", "2024-01-01T00:00:00+00:00", "cliente");
    assert!(matches!(decidir_promocao(&eu, &[outro], 2), Decisao::Aguardar(_)));
}

#[test]
fn empate_de_data_desempata_pelo_identificador_estavel() {
    let a = m("dev-a", JAN, "cliente");
    let b = m("dev-b", JAN, "cliente");
    assert_eq!(decidir_promocao(&a, &[b.clone()], 2), Decisao::Promover);
    assert!(matches!(decidir_promocao(&b, &[a.clone()], 2), Decisao::Aguardar(_)));
    // A ordem em que os computadores foram descobertos não muda o resultado.
    assert_eq!(decidir_promocao(&a, &[b.clone(), m("dev-c", MAR, "cliente")], 3), Decisao::Promover);
    assert_eq!(decidir_promocao(&a, &[m("dev-c", MAR, "cliente"), b], 3), Decisao::Promover);
}

#[test]
fn data_e_comparada_como_instante_nao_como_texto() {
    // 10:00 em UTC-03 é 13:00 UTC; 12:30 UTC é anterior.
    let eu = m("dev-a", "2024-01-01T10:00:00-03:00", "cliente");
    let outro = m("dev-b", "2024-01-01T12:30:00+00:00", "cliente");
    assert!(matches!(decidir_promocao(&eu, &[outro], 2), Decisao::Aguardar(_)));
}

#[test]
fn data_invalida_fica_por_ultimo() {
    let eu = m("dev-a", "", "cliente");
    let outro = m("dev-b", FEV, "cliente");
    assert!(matches!(decidir_promocao(&eu, &[outro], 2), Decisao::Aguardar(_)));
}

#[test]
fn computador_isolado_nao_e_candidato() {
    // Ex-servidor isolado é mais antigo, mas não assume sozinho; o próximo com quórum é este.
    let eu = m("dev-b", FEV, "cliente");
    let isolado = m("dev-a", JAN, "isolado");
    assert_eq!(decidir_promocao(&eu, &[isolado], 2), Decisao::Promover);
}

#[test]
fn quem_respondeu_duas_vezes_conta_uma_vez() {
    let eu = m("dev-b", FEV, "cliente");
    let a = m("dev-a", JAN, "cliente");
    // Sem deduplicar, 3 respostas pareceriam maioria de 4; são só 2 computadores.
    assert!(matches!(decidir_promocao(&eu, &[a.clone(), a], 4), Decisao::Aguardar(_)));
}

#[test]
fn modo_local_e_indefinido_nao_usam_rede() {
    assert!(!modo_usa_rede("local"));
    assert!(!modo_usa_rede(""));
    assert!(!modo_usa_rede("indefinido"));
    assert!(modo_usa_rede("servidor"));
    assert!(modo_usa_rede("cliente"));
}

#[test]
fn epoca_mais_recente_vence_a_antiguidade() {
    // "dev-a" foi ativado antes, mas tem dados de época 1; "dev-b" participou da época 2.
    let a = com_epoca("dev-a", JAN, "cliente", 1);
    let b = com_epoca("dev-b", FEV, "cliente", 2);
    assert_eq!(decidir_promocao(&b, &[a.clone(), m("dev-c", MAR, "cliente")], 3), Decisao::Promover);
    assert!(matches!(decidir_promocao(&a, &[b, m("dev-c", MAR, "cliente")], 3), Decisao::Aguardar(_)));
}

#[test]
fn servidor_que_volta_desatualizado_cede_e_o_atual_assume() {
    // Principal antigo (época 1) volta. O computador que assumiu na queda (época 2) está como cliente.
    let principal = com_epoca("dev-a", JAN, "servidor", 1);
    let atual = com_epoca("dev-b", FEV, "cliente", 2);
    assert_eq!(decidir_servidor(&principal, &[atual.clone()]), Decisao::Ceder);
    // Depois de ceder, o principal aparece como "sem_servidor" com a época antiga: o atual é promovido.
    let principal_cedido = com_epoca("dev-a", JAN, "sem_servidor", 1);
    assert_eq!(decidir_promocao(&atual, &[principal_cedido], 3), Decisao::Promover);
}

#[test]
fn servidor_atual_mantem_o_papel_diante_do_antigo() {
    let atual = com_epoca("dev-b", FEV, "servidor", 2);
    let antigo = com_epoca("dev-a", JAN, "cliente", 1);
    assert_eq!(decidir_servidor(&atual, &[antigo]), Decisao::Manter);
}

#[test]
fn servidor_cede_a_outro_servidor_de_epoca_maior() {
    let eu = com_epoca("dev-a", JAN, "servidor", 1);
    let outro = com_epoca("dev-b", FEV, "servidor", 2);
    assert_eq!(decidir_servidor(&eu, &[outro]), Decisao::Ceder);
}

#[test]
fn dois_servidores_da_mesma_epoca_decidem_pela_antiguidade() {
    let antigo = com_epoca("dev-a", JAN, "servidor", 3);
    let novo = com_epoca("dev-b", FEV, "servidor", 3);
    assert_eq!(decidir_servidor(&antigo, &[novo.clone()]), Decisao::Manter);
    assert_eq!(decidir_servidor(&novo, &[antigo]), Decisao::Ceder);
}

#[test]
fn servidor_nao_cede_a_quem_tem_epoca_menor() {
    let eu = com_epoca("dev-a", JAN, "servidor", 5);
    let atrasado = com_epoca("dev-b", JAN, "cliente", 4);
    assert_eq!(decidir_servidor(&eu, &[atrasado]), Decisao::Manter);
}

#[test]
fn proxima_epoca_passa_do_maior_conhecido() {
    let eu = com_epoca("dev-a", JAN, "cliente", 2);
    let outros = [com_epoca("dev-b", FEV, "cliente", 5), com_epoca("dev-c", MAR, "cliente", 3)];
    assert_eq!(proxima_epoca(&eu, &outros), 6);
    assert_eq!(proxima_epoca(&eu, &[]), 3);
}

#[test]
fn cliente_so_copia_de_servidor_com_epoca_igual_ou_mais_recente() {
    assert!(servidor_aceitavel(2, 2));
    assert!(servidor_aceitavel(2, 3));
    assert!(!servidor_aceitavel(2, 1));
}

#[test]
fn sobrevivente_sozinho_de_tres_nao_se_promove() {
    // 2 de 3 computadores fora do ar: 1 de 3 não é maioria. Regra de segurança (evita dois servidores em partições).
    let eu = com_epoca("dev-b", FEV, "cliente", 2);
    assert!(matches!(decidir_promocao(&eu, &[], 3), Decisao::Aguardar(_)));
}
