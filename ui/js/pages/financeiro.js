// Financeiro: o que falta receber das ordens e o que falta pagar das despesas.
// Os valores vêm em centavos (inteiros); aqui só convertemos para reais na tela.
import { chamar } from '../api.js';
import { el, aviso, botao, moeda, tabela, cabecalhoPagina, icone } from '../components.js';

const FORMAS = [['dinheiro', 'Dinheiro'], ['pix', 'PIX'], ['cartao', 'Cartão'], ['outro', 'Outra forma']];
const SITUACOES_RECEBER = [['', 'Todas'], ['ABERTA', 'Em aberto'], ['PARCIAL', 'Pago em parte'], ['QUITADA', 'Quitadas'], ['VENCIDA', 'Vencidas']];
const SITUACOES_PAGAR = [['', 'Todas'], ['ABERTA', 'Em aberto'], ['PARCIAL', 'Pago em parte'], ['PAGA', 'Pagas'], ['VENCIDA', 'Vencidas']];
const SITUACAO_TEXTO = { ABERTA: 'Em aberto', PARCIAL: 'Pago em parte', QUITADA: 'Quitada', PAGA: 'Paga', VENCIDA: 'Vencida' };

const reais = (centavos) => moeda((centavos ?? 0) / 100);

// O que a pessoa digita ("50", "50,00" ou "50.00") vira centavos inteiros. Valor inválido vira NaN.
function paraCentavos(texto) {
  const n = Number(String(texto ?? '').trim().replace(',', '.'));
  return Number.isFinite(n) ? Math.round(n * 100) : NaN;
}

// Data de hoje no formato AAAA-MM-DD, no horário local.
function hojeIso() {
  const d = new Date();
  const mes = String(d.getMonth() + 1).padStart(2, '0');
  const dia = String(d.getDate()).padStart(2, '0');
  return `${d.getFullYear()}-${mes}-${dia}`;
}

// Erros de validação chegam por campo; os demais, com o texto pronto.
function mensagemErro(e) {
  const campos = Object.values(e?.campos ?? {});
  return campos.length ? campos.join(' ') : (e?.texto ?? 'Algo deu errado. Tente de novo.');
}

function campoTexto(rotulo, input) {
  return el('label', { class: 'campo' }, el('span', { class: 'rotulo' }, rotulo), input);
}

function selecao(opcoes, valorInicial = '') {
  return el('select', {}, opcoes.map(([v, t]) => el('option', { value: v, selected: String(v) === String(valorInicial) ? '' : null }, t)));
}

function indicador(titulo, valor, classe = '') {
  return el('div', { class: `indicador-caixa ${classe}` }, el('span', { class: 'rotulo' }, titulo), el('strong', {}, valor));
}

export async function render(container, ctx = {}) {
  const estado = {
    aba: 'receber',
    filtro: { desde: '', ate: '', situacao: '' },
    // Formulário aberto: null, 'recebimento', 'pagamento' ou 'despesa'. `preselecionado` é a ordem ou conta da linha clicada.
    form: null,
    preselecionado: null,
  };

  // Filtros de período e situação, enviados igual para a lista e para o resumo.
  function argsFiltro() {
    const { desde, ate, situacao } = estado.filtro;
    return { desde: desde || null, ate: ate || null, situacao: situacao || null };
  }

  async function recarregar() {
    await desenhar();
  }

  function barraFiltros() {
    const desde = el('input', { type: 'date', value: estado.filtro.desde, 'aria-label': 'Data inicial' });
    const ate = el('input', { type: 'date', value: estado.filtro.ate, 'aria-label': 'Data final' });
    const situacao = selecao(estado.aba === 'receber' ? SITUACOES_RECEBER : SITUACOES_PAGAR, estado.filtro.situacao);
    situacao.setAttribute('aria-label', 'Situação');
    const aplicar = () => {
      estado.filtro = { desde: desde.value, ate: ate.value, situacao: situacao.value };
      recarregar();
    };
    desde.addEventListener('change', aplicar);
    ate.addEventListener('change', aplicar);
    situacao.addEventListener('change', aplicar);
    return el('section', { class: 'card' },
      el('h3', {}, icone('lupa', 24), 'Filtrar'),
      el('div', { class: 'acoes' },
        campoTexto('De', desde),
        campoTexto('Até', ate),
        campoTexto('Situação', situacao),
        botao({ texto: 'Limpar filtros', icone: 'x', tipo: 'secundario', onclick: () => {
          estado.filtro = { desde: '', ate: '', situacao: '' };
          recarregar();
        } })));
  }

  function formRecebimento(contas, preselecionado) {
    const abertas = contas.filter((c) => c.origem === 'OS' && c.saldo_centavos > 0);
    const ordem = el('select', { 'aria-label': 'Ordem que está pagando' },
      el('option', { value: '' }, '— escolha a ordem —'),
      abertas.map((c) => el('option', {
        value: c.referencia,
        selected: preselecionado && Number(preselecionado) === c.referencia ? '' : null,
      }, `Ordem nº ${c.referencia} · ${c.cliente_nome || 'sem cliente'} · falta ${reais(c.saldo_centavos)}`)));
    const valor = el('input', { type: 'number', step: '0.01', min: '0.01', placeholder: 'Ex.: 50,00', 'aria-label': 'Valor recebido em reais' });
    const forma = selecao(FORMAS, 'dinheiro');
    const salvar = async () => {
      if (!ordem.value) return aviso('Escolha a ordem que está pagando.', 'erro');
      const centavos = paraCentavos(valor.value);
      if (!(centavos > 0)) return aviso('Informe um valor maior que zero.', 'erro');
      try {
        await chamar('os_registrar_recebimento', { ordem_id: Number(ordem.value), valor_centavos: centavos, forma: forma.value });
        aviso('Recebimento registrado.');
        estado.form = null;
        recarregar();
      } catch (e) {
        aviso(mensagemErro(e), 'erro');
      }
    };
    return el('section', { class: 'card' },
      el('h3', {}, icone('caixa', 26), 'Registrar recebimento'),
      el('p', { class: 'ajuda' }, 'Pode ser um sinal, uma parte do valor ou o pagamento final. Se a pessoa pagar de mais de um jeito, registre cada parte.'),
      campoTexto('Ordem que está pagando', ordem),
      campoTexto('Valor recebido (R$)', valor),
      campoTexto('Forma de pagamento', forma),
      el('div', { class: 'acoes' },
        botao({ texto: 'Salvar recebimento', icone: 'ok', tipo: 'primario', onclick: salvar }),
        botao({ texto: 'Cancelar', icone: 'x', onclick: () => { estado.form = null; recarregar(); } })));
  }

  function formPagamento(contas, preselecionado) {
    const abertas = contas.filter((c) => c.saldo_centavos > 0);
    const conta = el('select', { 'aria-label': 'Conta que está sendo paga' },
      el('option', { value: '' }, '— escolha a conta —'),
      abertas.map((c) => el('option', {
        value: c.id,
        selected: preselecionado && Number(preselecionado) === c.id ? '' : null,
      }, `${c.descricao} · falta ${reais(c.saldo_centavos)}`)));
    const valor = el('input', { type: 'number', step: '0.01', min: '0.01', placeholder: 'Ex.: 100,00', 'aria-label': 'Valor pago em reais' });
    const data = el('input', { type: 'date', value: hojeIso(), 'aria-label': 'Data do pagamento' });
    const salvar = async () => {
      if (!conta.value) return aviso('Escolha a conta que está sendo paga.', 'erro');
      const centavos = paraCentavos(valor.value);
      if (!(centavos > 0)) return aviso('Informe um valor maior que zero.', 'erro');
      try {
        await chamar('contas_pagar_pagar', { id: Number(conta.value), valor_centavos: centavos, data: data.value || null });
        aviso('Pagamento registrado.');
        estado.form = null;
        recarregar();
      } catch (e) {
        aviso(mensagemErro(e), 'erro');
      }
    };
    return el('section', { class: 'card' },
      el('h3', {}, icone('seta', 24), 'Registrar pagamento'),
      el('p', { class: 'ajuda' }, 'Pode pagar uma parte agora e o restante depois.'),
      campoTexto('Conta que está sendo paga', conta),
      campoTexto('Valor pago (R$)', valor),
      campoTexto('Data do pagamento', data),
      el('div', { class: 'acoes' },
        botao({ texto: 'Salvar pagamento', icone: 'ok', tipo: 'primario', onclick: salvar }),
        botao({ texto: 'Cancelar', icone: 'x', onclick: () => { estado.form = null; recarregar(); } })));
  }

  function formDespesa() {
    const descricao = el('input', { type: 'text', maxlength: '200', placeholder: 'Ex.: Conta de luz', 'aria-label': 'O que está sendo pago' });
    const fornecedor = el('input', { type: 'text', placeholder: 'Opcional', 'aria-label': 'Fornecedor' });
    const valor = el('input', { type: 'number', step: '0.01', min: '0.01', placeholder: 'Ex.: 150,00', 'aria-label': 'Valor em reais' });
    const vencimento = el('input', { type: 'date', value: hojeIso(), 'aria-label': 'Data de vencimento' });
    const salvar = async () => {
      const centavos = paraCentavos(valor.value);
      try {
        await chamar('contas_pagar_criar', { dados: {
          descricao: descricao.value,
          fornecedor: fornecedor.value || null,
          valor_centavos: Number.isFinite(centavos) ? centavos : 0,
          vencimento: vencimento.value,
        } });
        aviso('Despesa cadastrada.');
        estado.form = null;
        recarregar();
      } catch (e) {
        aviso(mensagemErro(e), 'erro');
      }
    };
    return el('section', { class: 'card' },
      el('h3', {}, icone('mais', 24), 'Cadastrar despesa'),
      campoTexto('O que está sendo pago', descricao),
      campoTexto('Fornecedor (opcional)', fornecedor),
      campoTexto('Valor (R$)', valor),
      campoTexto('Vencimento', vencimento),
      el('div', { class: 'acoes' },
        botao({ texto: 'Salvar despesa', icone: 'ok', tipo: 'primario', onclick: salvar }),
        botao({ texto: 'Cancelar', icone: 'x', onclick: () => { estado.form = null; recarregar(); } })));
  }

  function abaReceber(contas, resumo) {
    const linhas = contas.map((c) => ({
      ...c,
      ordem_texto: c.origem === 'OS' ? `Ordem nº ${c.referencia}` : `Venda nº ${c.referencia}`,
      total_texto: reais(c.valor_total_centavos),
      recebido_texto: reais(c.recebido_centavos),
      saldo_texto: reais(c.saldo_centavos),
      vencimento_texto: c.vencimento ? c.vencimento.split('-').reverse().join('/') : '',
      situacao_texto: SITUACAO_TEXTO[c.situacao] ?? c.situacao,
      __acoes: c.origem === 'OS' && c.saldo_centavos > 0
        ? botao({ texto: 'Receber', tipo: 'link', title: `Registrar recebimento da ordem ${c.referencia}`,
          onclick: () => { estado.form = 'recebimento'; estado.preselecionado = c.referencia; recarregar(); } })
        : null,
    }));
    return [
      el('div', { class: 'indicadores-caixa' },
        indicador('Valor faturado', reais(resumo.faturado_centavos)),
        indicador('Valor recebido', reais(resumo.recebido_centavos)),
        indicador('Saldo pendente', reais(resumo.pendente_centavos), 'destaque')),
      el('div', { class: 'acoes' },
        botao({ texto: 'Registrar recebimento', icone: 'caixa', tipo: 'primario', onclick: () => { estado.form = 'recebimento'; estado.preselecionado = null; recarregar(); } })),
      estado.form === 'recebimento' ? formRecebimento(contas, estado.preselecionado) : null,
      tabela([
        { key: 'ordem_texto', label: 'Ordem' },
        { key: 'cliente_nome', label: 'Cliente' },
        { key: 'total_texto', label: 'Valor da ordem' },
        { key: 'recebido_texto', label: 'Já recebido' },
        { key: 'saldo_texto', label: 'Falta receber' },
        { key: 'vencimento_texto', label: 'Vence em' },
        { key: 'situacao_texto', label: 'Situação' },
      ], linhas, 'Nenhuma conta a receber neste filtro.'),
    ];
  }

  function abaPagar(contas, resumo) {
    const linhas = contas.map((c) => ({
      ...c,
      fornecedor_texto: c.fornecedor || '—',
      valor_texto: reais(c.valor_centavos),
      pago_texto: reais(c.pago_centavos),
      saldo_texto: reais(c.saldo_centavos),
      vencimento_texto: c.vencimento ? c.vencimento.split('-').reverse().join('/') : '',
      situacao_texto: SITUACAO_TEXTO[c.situacao] ?? c.situacao,
      __acoes: c.saldo_centavos > 0
        ? botao({ texto: 'Pagar', tipo: 'link', title: `Registrar pagamento de ${c.descricao}`,
          onclick: () => { estado.form = 'pagamento'; estado.preselecionado = c.id; recarregar(); } })
        : null,
    }));
    return [
      el('div', { class: 'indicadores-caixa' },
        indicador('Total das despesas', reais(resumo.a_pagar_centavos)),
        indicador('Já pago', reais(resumo.pago_centavos)),
        indicador('Falta pagar', reais(resumo.a_pagar_pendente_centavos), 'destaque')),
      el('div', { class: 'acoes' },
        botao({ texto: 'Registrar pagamento', icone: 'seta', tipo: 'primario', onclick: () => { estado.form = 'pagamento'; estado.preselecionado = null; recarregar(); } }),
        botao({ texto: 'Cadastrar despesa', icone: 'mais', tipo: 'secundario', onclick: () => { estado.form = 'despesa'; recarregar(); } })),
      estado.form === 'pagamento' ? formPagamento(contas, estado.preselecionado) : null,
      estado.form === 'despesa' ? formDespesa() : null,
      tabela([
        { key: 'descricao', label: 'Descrição' },
        { key: 'fornecedor_texto', label: 'Fornecedor' },
        { key: 'valor_texto', label: 'Valor' },
        { key: 'pago_texto', label: 'Já pago' },
        { key: 'saldo_texto', label: 'Falta pagar' },
        { key: 'vencimento_texto', label: 'Vence em' },
        { key: 'situacao_texto', label: 'Situação' },
      ], linhas, 'Nenhuma despesa neste filtro.'),
    ];
  }

  async function desenhar() {
    const args = argsFiltro();
    const abas = el('div', { class: 'acoes', role: 'group', 'aria-label': 'Escolha a lista' },
      botao({ texto: 'Receber', icone: 'caixa', tipo: estado.aba === 'receber' ? 'primario' : 'secundario', onclick: () => {
        estado.aba = 'receber'; estado.form = null; estado.filtro.situacao = ''; recarregar();
      } }),
      botao({ texto: 'Pagar', icone: 'seta', tipo: estado.aba === 'pagar' ? 'primario' : 'secundario', onclick: () => {
        estado.aba = 'pagar'; estado.form = null; estado.filtro.situacao = ''; recarregar();
      } }));

    let corpo;
    try {
      const [resumo, contas] = await Promise.all([
        chamar('financeiro_resumo', args),
        chamar(estado.aba === 'receber' ? 'contas_receber_listar' : 'contas_pagar_listar', args),
      ]);
      corpo = estado.aba === 'receber' ? abaReceber(contas, resumo) : abaPagar(contas, resumo);
    } catch (e) {
      corpo = [el('p', { class: 'vazio', role: 'alert' }, mensagemErro(e))];
    }

    container.replaceChildren(
      cabecalhoPagina('Financeiro', 'caixa', 'Veja o que falta receber das ordens e o que falta pagar das despesas.'),
      abas,
      barraFiltros(),
      ...corpo,
    );
  }

  await desenhar();
}
