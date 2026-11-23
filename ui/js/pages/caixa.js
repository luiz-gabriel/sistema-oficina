// Caixa: abrir com o troco, registrar sangrias e suprimentos, acompanhar o que entrou e fechar conferindo a gaveta.
import { chamar, invocar } from '../api.js';
import { el, aviso, botao, moeda, tabela, confirmar, cabecalhoPagina, icone } from '../components.js';

const dataHora = (texto) => (texto ? new Date(texto).toLocaleString('pt-BR') : '');
const FORMAS = { dinheiro: 'Dinheiro', pix: 'PIX', cartao: 'Cartão', outro: 'Outra forma' };
const MOVIMENTO_COLUNAS = [
  { key: 'hora', label: 'Hora' },
  { key: 'tipo_texto', label: 'Tipo' },
  { key: 'valor_texto', label: 'Valor' },
  { key: 'motivo', label: 'Motivo' },
  { key: 'usuario', label: 'Feito por' },
];

const movimentoLinha = (m) => ({
  ...m,
  hora: dataHora(m.criado_em),
  tipo_texto: m.tipo === 'sangria' ? 'Sangria' : 'Suprimento',
  valor_texto: moeda(m.valor),
});

function indicador(titulo, valor, classe = '') {
  return el('div', { class: `indicador-caixa ${classe}` }, el('span', { class: 'rotulo' }, titulo), el('strong', {}, valor));
}

// Erros de validação chegam com a mensagem de cada campo; os demais, com o texto pronto.
function mensagemErro(e) {
  const campos = Object.values(e?.campos ?? {});
  return campos.length ? campos.join(' ') : (e?.texto ?? 'Algo deu errado.');
}

function resultadoFechamento(rel) {
  const dif = Number(rel.diferenca ?? 0);
  const texto = Math.abs(dif) < 0.005
    ? 'Caixa conferido. O dinheiro da gaveta bate com o esperado.'
    : dif < 0
      ? `Faltou ${moeda(-dif)} na gaveta. Confira as anotações antes de sair.`
      : `Sobrou ${moeda(dif)} na gaveta. Anote o motivo na observação do caixa.`;
  return el('div', { class: `faixa ${Math.abs(dif) < 0.005 ? 'info' : 'alerta'}`, role: 'status' },
    icone(Math.abs(dif) < 0.005 ? 'ok' : 'alerta', 26), el('p', {}, texto));
}

function formAbrir(ctx) {
  const valor = el('input', { type: 'number', step: '0.01', min: '0', value: '0', 'aria-label': 'Dinheiro inicial na gaveta' });
  return el('section', { class: 'card' },
    el('h3', {}, icone('caixa', 26), 'Abrir o caixa'),
    el('p', { class: 'ajuda' }, 'Conte o dinheiro que já está na gaveta (troco). Se não houver nenhum, deixe 0.'),
    el('label', { class: 'campo' }, el('span', { class: 'rotulo' }, 'Dinheiro inicial na gaveta (R$)'), valor),
    el('div', { class: 'acoes' }, botao({ texto: 'Abrir o caixa agora', icone: 'ok', tipo: 'primario', onclick: async () => {
      try {
        await chamar('caixa_abrir', { valor_inicial: Number(valor.value || 0) });
        aviso('Caixa aberto.');
        ctx.irPara('caixa');
        ctx.atualizarEstado?.();
      } catch (e) { aviso(mensagemErro(e), 'erro'); }
    } })));
}

// Sangria (tirar dinheiro) ou suprimento (colocar dinheiro) no caixa aberto. `ajustar` redesenha a tela.
function formMovimento(tipo, ajustar) {
  const sangria = tipo === 'sangria';
  const comando = sangria ? 'caixa_sangria' : 'caixa_suprimento';
  const valor = el('input', { type: 'number', step: '0.01', min: '0.01', 'aria-label': 'Valor' });
  const motivo = el('input', {
    type: 'text', maxlength: 120, 'aria-label': 'Motivo',
    placeholder: sangria ? 'Ex.: depósito no banco, pagamento de fornecedor' : 'Ex.: reforço de troco',
  });
  return el('section', { class: 'card' },
    el('h3', {}, icone(sangria ? 'seta' : 'mais', 26), sangria ? 'Sangria: retirar dinheiro da gaveta' : 'Suprimento: colocar dinheiro na gaveta'),
    el('p', { class: 'ajuda' }, sangria
      ? 'Use quando tirar dinheiro da gaveta durante o expediente. Esse valor sai do dinheiro esperado.'
      : 'Use quando colocar dinheiro na gaveta durante o expediente. Esse valor entra no dinheiro esperado.'),
    el('label', { class: 'campo' }, el('span', { class: 'rotulo' }, 'Valor (R$)'), valor),
    el('label', { class: 'campo' }, el('span', { class: 'rotulo' }, 'Motivo (obrigatório)'), motivo),
    el('div', { class: 'acoes' },
      botao({ texto: 'Registrar', icone: 'ok', tipo: 'primario', onclick: async () => {
        const v = Number(valor.value);
        if (!(v > 0)) return aviso('Informe um valor maior que zero.', 'erro');
        if (!motivo.value.trim()) return aviso('Informe o motivo.', 'erro');
        try {
          await chamar(comando, { valor: v, motivo: motivo.value });
          aviso(sangria ? 'Sangria registrada.' : 'Suprimento registrado.');
          await ajustar({ formMov: null });
        } catch (e) { aviso(mensagemErro(e), 'erro'); }
      } }),
      botao({ texto: 'Cancelar', tipo: 'secundario', onclick: () => ajustar({ formMov: null }) })));
}

function formFechar(s, ajustar) {
  const contado = el('input', { type: 'number', step: '0.01', min: '0', required: '', 'aria-label': 'Dinheiro contado na gaveta' });
  const obs = el('textarea', { rows: 2, placeholder: 'Opcional: ex.: troco separado, diferença explicada' }, '');
  return el('section', { class: 'card' },
    el('h3', {}, icone('lapis', 26), 'Fechar o caixa'),
    el('p', { class: 'texto-grande' }, `Troco inicial (dinheiro que abriu o caixa): ${moeda(s.sessao.valor_abertura)}.`),
    el('p', { class: 'ajuda' }, `Esperado em dinheiro na gaveta: ${moeda(s.dinheiro_esperado)} = troco inicial + dinheiro recebido + suprimentos - sangrias.`),
    el('label', { class: 'campo' }, el('span', { class: 'rotulo' }, 'Dinheiro contado na gaveta (R$)'), contado),
    el('label', { class: 'campo' }, el('span', { class: 'rotulo' }, 'Observação'), obs),
    el('div', { class: 'acoes' }, botao({ texto: 'Fechar o caixa e conferir', icone: 'cadeado', tipo: 'primario', onclick: async () => {
      if (contado.value === '') return aviso('Conte o dinheiro e informe o valor.', 'erro');
      if (!confirmar('Fechar o caixa agora? Depois disso não há mais recebimentos nem movimentos neste caixa.')) return;
      try {
        await chamar('caixa_fechar', { id: s.sessao.id, valor_contado: Number(contado.value), observacao: obs.value });
        aviso('Caixa fechado.');
        // O fechamento já foi gravado: se o resumo falhar, a tela ainda atualiza.
        const relatorio = await chamar('caixa_relatorio', { id: s.sessao.id }).catch(() => null);
        await ajustar({ formMov: null, ultimo: relatorio });
      } catch (e) { aviso(mensagemErro(e), 'erro'); }
    } })));
}

// Resumo completo de um caixa fechado (logo após fechar, ou pelo histórico).
function resumoCaixa(rel, ajustar) {
  const r = rel.recebimentos;
  const fechado = Boolean(rel.sessao.fechado_em);
  const indicadores = [
    ['Troco inicial', moeda(rel.sessao.valor_abertura)],
    ['Dinheiro recebido', moeda(r.dinheiro)],
    ['PIX', moeda(r.pix)],
    ['Cartão', moeda(r.cartao)],
    ['Outra forma', moeda(r.outro)],
    ['Total recebido', moeda(r.total)],
    ['Sangrias', moeda(rel.sangrias)],
    ['Suprimentos', moeda(rel.suprimentos)],
    ['Dinheiro esperado', moeda(rel.dinheiro_esperado)],
    ['Dinheiro contado', rel.dinheiro_contado == null ? '—' : moeda(rel.dinheiro_contado)],
    ['Diferença', rel.diferenca == null ? '—' : moeda(rel.diferenca)],
    ['Atendimentos', String(r.atendimentos)],
  ];
  // Uma linha por venda ou OS: tipo, cliente, total cobrado, o que entrou neste caixa e as formas.
  const vendas = rel.recebidos.map((o) => ({
    ...o,
    tipo_texto: o.tipo === 'BALCAO' ? 'Venda de balcão' : 'Ordem de serviço',
    ordem: `${o.tipo === 'BALCAO' ? 'Venda' : 'OS'} nº ${o.os_id}`,
    cliente_texto: o.cliente || '—',
    total_texto: moeda(o.total_venda),
    recebido_texto: moeda(o.valor),
    forma: (o.pagamento || '').split(',').map((f) => FORMAS[f.trim()] ?? f.trim()).join(', ') || '—',
    hora: dataHora(o.concluido_em),
  }));
  return el('section', { class: 'card', 'aria-label': 'Resumo do caixa' },
    el('h3', {}, icone('registro', 26), `Resumo do caixa ${rel.sessao.id}`),
    el('p', { class: 'texto-grande' }, `Aberto em ${dataHora(rel.sessao.aberto_em)} por ${rel.sessao.aberto_por}` +
      (fechado ? `. Fechado em ${dataHora(rel.sessao.fechado_em)} por ${rel.sessao.fechado_por}.` : '. Ainda aberto.')),
    fechado ? resultadoFechamento(rel) : null,
    el('div', { class: 'indicadores-caixa' }, indicadores.map(([t, v]) => indicador(t, v, t === 'Diferença' ? 'destaque' : ''))),
    el('h4', {}, 'Vendas e ordens recebidas neste caixa'),
    tabela([
      { key: 'tipo_texto', label: 'Tipo' },
      { key: 'ordem', label: 'Número' },
      { key: 'cliente_texto', label: 'Cliente' },
      { key: 'total_texto', label: 'Total da venda' },
      { key: 'recebido_texto', label: 'Recebido neste caixa' },
      { key: 'forma', label: 'Forma de pagamento' },
      { key: 'hora', label: 'Recebido em' },
    ], vendas, 'Nenhuma venda ou ordem recebida neste caixa.'),
    el('h4', {}, 'Sangrias e suprimentos'),
    tabela(MOVIMENTO_COLUNAS, rel.movimentos.map(movimentoLinha), 'Nenhuma sangria ou suprimento neste caixa.'),
    el('div', { class: 'acoes' },
      botao({ texto: 'Imprimir fechamento', icone: 'registro', tipo: 'primario', onclick: () => imprimir(rel.sessao.id) }),
      ajustar ? botao({ texto: 'Fechar este resumo', icone: 'x', onclick: () => ajustar({ ultimo: null }) }) : null));
}

// Envia o fechamento à impressora configurada. Sem impressora, a mensagem diz onde configurar; o caixa não muda.
async function imprimir(id) {
  try {
    await invocar('caixa_imprimir', { id });
    aviso('Fechamento enviado para a impressora.');
  } catch (e) {
    aviso(mensagemErro(e), 'erro');
  }
}

export async function render(container, ctx) {
  // Estado da tela: formulário de sangria/suprimento aberto e resumo mostrado no topo.
  const estado = { formMov: null, ultimo: null };

  async function ajustar(mudanca = {}) {
    Object.assign(estado, mudanca);
    await desenhar();
  }

  async function desenhar() {
    const admin = Boolean(ctx.admin);
    // Histórico e resultados de caixas anteriores são só do administrador.
    const [s, historico] = await Promise.all([chamar('caixa_situacao'), admin ? chamar('caixa_historico', { limite: 15 }) : Promise.resolve([])]);
    const partes = [cabecalhoPagina('Caixa', 'caixa', 'Abra o caixa no começo do dia, registre sangrias e suprimentos, e feche no fim conferindo o dinheiro da gaveta.')];

    if (estado.ultimo) partes.push(resumoCaixa(estado.ultimo, ajustar));

    if (s.aberto && s.de_outro_operador) {
      // Caixa de outro operador: sem valores. Só quem abriu ou o administrador fecha.
      partes.push(el('section', { class: 'card', role: 'status' },
        el('h3', {}, icone('caixa', 26), 'Caixa aberto por outra pessoa'),
        el('p', { class: 'texto-grande' }, `O caixa foi aberto por ${s.sessao.aberto_por} em ${dataHora(s.sessao.aberto_em)}.`),
        el('p', { class: 'ajuda' }, 'Só quem abriu o caixa ou o administrador pode fechá-lo. Para abrir o seu, peça que ele feche antes.')));
    } else if (s.aberto) {
      const r = s.recebimentos;
      partes.push(el('section', { class: 'card' },
        el('h3', {}, icone('caixa', 26), 'Caixa aberto'),
        el('p', { class: 'texto-grande' }, `Aberto em ${dataHora(s.sessao.aberto_em)} por ${s.sessao.aberto_por}. Troco inicial: ${moeda(s.sessao.valor_abertura)}.`),
        el('div', { class: 'indicadores-caixa' },
          indicador('Dinheiro', moeda(r.dinheiro)),
          indicador('PIX', moeda(r.pix)),
          indicador('Cartão', moeda(r.cartao)),
          indicador('Outra forma', moeda(r.outro)),
          indicador('Sangrias', moeda(s.sangrias)),
          indicador('Suprimentos', moeda(s.suprimentos)),
          indicador('Total recebido', moeda(r.total), 'destaque'),
          indicador('Dinheiro esperado', moeda(s.dinheiro_esperado)),
          indicador('Atendimentos', String(r.atendimentos)))));

      partes.push(el('section', { class: 'card' },
        el('h3', {}, icone('registro', 24), 'Imprimir valores do caixa'),
        el('p', { class: 'ajuda' }, 'Imprime o que já entrou e saiu deste caixa, sem fechar. Útil para conferir no meio do dia.'),
        el('div', { class: 'acoes' }, botao({ texto: 'Imprimir valores do caixa agora', icone: 'registro', tipo: 'primario', onclick: () => imprimir(s.sessao.id) }))));

      partes.push(el('section', { class: 'card' },
        el('h3', {}, icone('seta', 24), 'Movimentos da gaveta'),
        el('p', { class: 'ajuda' }, 'Sangria tira dinheiro da gaveta. Suprimento coloca dinheiro. Os dois entram no dinheiro esperado do fechamento.'),
        el('div', { class: 'acoes' },
          botao({ texto: 'Sangria', icone: 'seta', tipo: 'secundario', title: 'Retirar dinheiro da gaveta', onclick: () => ajustar({ formMov: 'sangria' }) }),
          botao({ texto: 'Suprimento', icone: 'mais', tipo: 'secundario', title: 'Colocar dinheiro na gaveta', onclick: () => ajustar({ formMov: 'suprimento' }) })),
        estado.formMov ? formMovimento(estado.formMov, ajustar) : null,
        tabela(MOVIMENTO_COLUNAS, s.movimentos.map(movimentoLinha), 'Nenhuma sangria ou suprimento neste caixa.')));

      partes.push(formFechar(s, ajustar));
    } else {
      partes.push(formAbrir(ctx));
    }

    if (!admin) return container.replaceChildren(...partes);
    const linhas = historico.map((h) => ({
      ...h,
      abriu: dataHora(h.aberto_em),
      fechou: h.fechado_em ? dataHora(h.fechado_em) : 'Aberto agora',
      diferenca_texto: h.diferenca === null || h.diferenca === undefined ? '—' : moeda(h.diferenca),
      __acoes: el('span', {},
        botao({ texto: 'Ver resumo', tipo: 'link', onclick: async () => ajustar({ ultimo: await chamar('caixa_relatorio', { id: h.id }) }) }),
        botao({ texto: 'Imprimir', tipo: 'link', onclick: () => imprimir(h.id) })),
    }));
    partes.push(el('section', { class: 'card' },
      el('h3', {}, icone('registro', 24), 'Caixas anteriores'),
      tabela([
        { key: 'abriu', label: 'Aberto em' },
        { key: 'aberto_por', label: 'Aberto por' },
        { key: 'fechou', label: 'Fechado em' },
        { key: 'fechado_por', label: 'Fechado por' },
        { key: 'diferenca_texto', label: 'Diferença' },
      ], linhas, 'Nenhum caixa aberto ainda.')));
    container.replaceChildren(...partes);
  }

  await desenhar();
}
