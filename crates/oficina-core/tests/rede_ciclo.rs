// Ciclo de rede (escuta e sincronização): rede → local → rede inicia e encerra de forma correta.
use oficina_core::sync::CicloRede;

#[test]
fn rede_local_rede_inicia_encerra_e_reinicia() {
    let mut ciclo = CicloRede::novo();
    assert!(!ciclo.ligado());

    // Modo rede: liga uma vez.
    let primeira = ciclo.iniciar().expect("liga a rede");
    assert!(ciclo.vigente(primeira));
    assert!(ciclo.iniciar().is_none(), "não pode iniciar duas vezes");

    // Modo local: encerra. A execução antiga deixa de ser vigente e não sobra nada ligado.
    ciclo.parar();
    assert!(!ciclo.ligado());
    assert!(!ciclo.vigente(primeira));

    // Volta para a rede: começa uma execução nova, e só ela é vigente.
    let segunda = ciclo.iniciar().expect("religa a rede depois do modo local");
    assert_ne!(segunda, primeira);
    assert!(ciclo.vigente(segunda));
    assert!(!ciclo.vigente(primeira));
}

#[test]
fn parar_sem_ter_ligado_e_seguro() {
    let mut ciclo = CicloRede::novo();
    ciclo.parar();
    assert!(!ciclo.ligado());
    let g = ciclo.iniciar().expect("liga depois de parar sem ligar");
    assert!(ciclo.vigente(g));
}

#[test]
fn varias_trocas_de_modo_deixam_so_a_ultima_execucao_vigente() {
    let mut ciclo = CicloRede::novo();
    let mut geracoes = Vec::new();
    for _ in 0..3 {
        geracoes.push(ciclo.iniciar().unwrap());
        ciclo.parar();
    }
    let ultima = ciclo.iniciar().unwrap();
    assert!(geracoes.iter().all(|g| !ciclo.vigente(*g)));
    assert!(ciclo.vigente(ultima));
}
