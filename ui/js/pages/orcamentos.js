// Orçamentos: proposta para o cliente aprovar antes do serviço.
// Só depois da aprovação o orçamento vira ordem de serviço. Peças extras durante o serviço entram como "adicional".
import { chamar, invocar } from '../api.js';
import { el, aviso, botao, campo, icone, moeda, tabela, confirmar, cabecalhoPagina } from '../components.js';

const ROTULOS = {
  RASCUNHO: 'Rascunho (ainda não enviado)',
  AGUARDANDO_APROVACAO: 'Aguardando aprovação do cliente',
  APROVADO: 'Aprovado',
  RECUSADO: 'Recusado pelo cliente',
  CANCELADO: 'Cancelado',
};

const FORMAS = [
  ['presencial', 'Presencialmente, na oficina'],
  ['telefone', 'Por telefone'],
  ['WhatsApp', 'Pelo WhatsApp'],
  ['outro', 'Outra forma (escreva abaixo)'],
];

const ROTULO_FORMA = Object.fromEntries(FORMAS);

const centavosParaMoeda = (c) => moeda((c ?? 0) / 100);
const dataBr = (iso) => (iso ? iso.split('-').reverse().join('/') : '—');
const dataHora = (texto) => (texto ? new Date(texto).toLocaleString('pt-BR') : '—');
// Data de hoje mais N dias, no formato AAAA-MM-DD (fuso local).
const dataDaqui = (dias) => {
  const d = new Date();
  d.setDate(d.getDate() + dias);
  return d.toLocaleDateString('en-CA');
};

export async function render(container) {
  const formArea = el('div', { class: 'card' });
  const listaArea = el('div', { class: 'card' });
  const detalheArea = el('div', { class: 'card' });
  container.replaceChildren(
    cabecalhoPagina('Orçamentos', 'ordens', 'Faça o orçamento, envie ao cliente para aprovar e só então transforme em ordem de serviço.'),
    formArea,
    listaArea,
    detalheArea,
  );

  // Seletor de peças e serviços: busca pelo nome, lista de itens escolhidos e total previsto.
  async function seletorItens(catalogo) {
    const porId = Object.fromEntries(catalogo.map((i) => [i.id, i]));
    let linhas = [];
    const busca = el('input', { type: 'search', autocomplete: 'off', placeholder: 'Digite o nome da peça ou do serviço', 'aria-label': 'Buscar peça ou serviço' });
    const resultados = el('div', { class: 'resultados-busca' });
    const lista = el('div', { class: 'itens-os' });
    const total = el('p', { class: 'total-orcamento' });

    function atualizarTotal() {
      const soma = linhas.reduce((s, l) => s + Math.round((porId[l.item_id]?.preco_venda ?? 0) * 100) * l.quantidade, 0);
      total.replaceChildren(el('strong', {}, `Total previsto: ${centavosParaMoeda(soma)}`));
    }

    function desenharLinhas() {
      lista.replaceChildren(...linhas.map((l, idx) => {
        const item = porId[l.item_id];
        const qtd = el('input', {
          type: 'number', min: 1, value: l.quantidade, 'aria-label': `Quantidade de ${item?.nome ?? 'item'}`,
          onchange: (ev) => {
            l.quantidade = Math.max(1, Number(ev.target.value) || 1);
            ev.target.value = l.quantidade;
            atualizarTotal();
          },
        });
        return el('div', { class: 'linha-item' },
          el('span', { class: 'nome-item' }, `${item?.nome ?? 'Item'} (${item?.tipo === 'servico' ? 'serviço' : 'peça'}) — ${moeda(item?.preco_venda)} cada`),
          el('label', {}, 'Quantidade ', qtd),
          el('button', { type: 'button', class: 'link', onclick: () => { linhas.splice(idx, 1); desenharLinhas(); } }, 'remover'));
      }));
      if (!linhas.length) lista.replaceChildren(el('p', { class: 'vazio' }, 'Nenhum item ainda. Use a busca acima para incluir peças e serviços.'));
      atualizarTotal();
    }

    busca.addEventListener('input', () => {
      const termo = busca.value.trim().toLowerCase();
      resultados.replaceChildren();
      if (!termo) return;
      const achados = catalogo.filter((i) => i.nome.toLowerCase().includes(termo)).slice(0, 8);
      if (!achados.length) resultados.append(el('p', { class: 'vazio' }, 'Nada encontrado com esse nome.'));
      for (const item of achados) {
        resultados.append(botao({
          texto: `${item.nome} — ${moeda(item.preco_venda)}`,
          icone: 'mais',
          onclick: () => {
            const existente = linhas.find((l) => l.item_id === item.id);
            if (existente) existente.quantidade += 1;
            else linhas.push({ item_id: item.id, quantidade: 1 });
            busca.value = '';
            resultados.replaceChildren();
            desenharLinhas();
            busca.focus();
            aviso(`${item.nome} incluído.`);
          },
        }));
      }
    });

    desenharLinhas();
    return {
      node: el('div', { class: 'seletor-itens' },
        el('label', { class: 'campo' }, el('span', { class: 'rotulo' }, 'Peças e serviços do orçamento'), busca),
        resultados, lista, total),
      get: () => linhas.map((l) => ({ item_id: l.item_id, quantidade: l.quantidade })),
    };
  }

  async function carregarBase() {
    const [clientes, veiculos, funcionarios, itens] = await Promise.all([
      chamar('clientes_listar'), chamar('veiculos_listar'), chamar('funcionarios_listar'), chamar('itens_listar'),
    ]);
    return { clientes, veiculos, funcionarios, itens };
  }

  async function mostrarNovo() {
    detalheArea.replaceChildren();
    const base = await carregarBase();
    if (!base.clientes.length || !base.veiculos.length) {
      formArea.replaceChildren(el('h3', {}, 'Novo orçamento'),
        el('p', { class: 'faixa alerta', role: 'alert' }, 'Antes de fazer o orçamento, cadastre o cliente e o veículo dele na tela Clientes e Veículos.'));
      return;
    }
    const seletor = await seletorItens(base.itens);
    const selCliente = campo({ name: 'cliente_id', label: 'Cliente *', type: 'select', required: true,
      options: base.clientes.map((c) => [c.id, c.nome]) });
    const selVeiculo = campo({ name: 'veiculo_id', label: 'Veículo do cliente *', type: 'select', required: true, options: [] });
    const selVeiculoEl = selVeiculo.querySelector('select');
    const preencherVeiculos = (clienteId) => {
      const doCliente = base.veiculos.filter((v) => String(v.cliente_id) === String(clienteId));
      selVeiculoEl.replaceChildren(el('option', { value: '' }, '— escolha —'),
        ...doCliente.map((v) => el('option', { value: v.id }, `${v.placa} — ${v.modelo}`)));
    };
    selCliente.querySelector('select').addEventListener('change', (ev) => preencherVeiculos(ev.target.value));
    preencherVeiculos('');

    const erroArea = el('p', { class: 'erro-campo', role: 'alert' });
    const form = el('form', {
      onsubmit: async (ev) => {
        ev.preventDefault();
        const d = Object.fromEntries(new FormData(ev.target).entries());
        erroArea.textContent = '';
        try {
          const orc = await chamar('orcamento_criar', {
            dados: {
              cliente_id: d.cliente_id ? Number(d.cliente_id) : null,
              veiculo_id: d.veiculo_id ? Number(d.veiculo_id) : null,
              funcionario_id: d.funcionario_id ? Number(d.funcionario_id) : null,
              validade: d.validade || null,
              prazo_dias: d.prazo_dias ? Number(d.prazo_dias) : null,
              observacoes: d.observacoes || null,
              itens: seletor.get(),
            },
          });
          aviso('Orçamento salvo como rascunho. Agora envie ao cliente para aprovar.');
          formArea.replaceChildren();
          await carregarLista();
          mostrarDetalhe(orc.id);
        } catch (e) {
          erroArea.textContent = Object.values(e.campos ?? {}).join(' ') || e.texto;
        }
      },
    },
      el('p', { class: 'ajuda' }, 'Nada entra na ordem de serviço antes de o cliente aprovar o orçamento.'),
      selCliente,
      selVeiculo,
      campo({ name: 'funcionario_id', label: 'Responsável pelo serviço *', type: 'select', required: true,
        options: base.funcionarios.map((f) => [f.id, f.nome]) }),
      seletor.node,
      el('div', { class: 'linha-item' },
        campo({ name: 'validade', label: 'Válido até (data limite) *', type: 'date', required: true, min: dataDaqui(0) }, dataDaqui(30)),
        campo({ name: 'prazo_dias', label: 'Prazo estimado do serviço (dias)', type: 'number', min: 0 }, '')),
      campo({ name: 'observacoes', label: 'Observações (aparecem no orçamento)', type: 'textarea' }, ''),
      erroArea,
      el('div', { class: 'acoes' },
        el('button', { type: 'submit', class: 'primario' }, 'Salvar orçamento'),
        el('button', { type: 'button', class: 'link', onclick: () => formArea.replaceChildren() }, 'cancelar')),
    );
    formArea.replaceChildren(el('h3', {}, 'Novo orçamento'), form);
    selCliente.querySelector('select').focus();
  }

  // Adicional: peça ou serviço extra numa OS que está em andamento. Só entra na OS depois de aprovado.
  async function mostrarAdicional() {
    detalheArea.replaceChildren();
    const [ordens, base] = await Promise.all([chamar('ordens_listar'), carregarBase()]);
    const abertas = ordens.filter((o) => o.tipo === 'OS' && ['CRIADA', 'EM_ANDAMENTO'].includes(o.status));
    if (!abertas.length) {
      formArea.replaceChildren(el('h3', {}, 'Adicional durante o serviço'),
        el('p', { class: 'vazio' }, 'Não há ordem de serviço criada ou em andamento.'));
      return;
    }
    const seletor = await seletorItens(base.itens);
    const erroArea = el('p', { class: 'erro-campo', role: 'alert' });
    const form = el('form', {
      onsubmit: async (ev) => {
        ev.preventDefault();
        const d = Object.fromEntries(new FormData(ev.target).entries());
        erroArea.textContent = '';
        try {
          const novo = await chamar('orcamento_adicional', {
            dados: { os_id: Number(d.os_id), observacoes: d.observacoes || null, itens: seletor.get() },
          });
          aviso('Adicional criado. Ele só entra na ordem depois que o cliente aprovar.');
          formArea.replaceChildren();
          await carregarLista();
          mostrarDetalhe(novo.id);
        } catch (e) {
          erroArea.textContent = Object.values(e.campos ?? {}).join(' ') || e.texto;
        }
      },
    },
      el('p', { class: 'ajuda' }, 'Use quando o cliente autorizar uma peça ou serviço a mais durante o serviço. Antes da aprovação, a ordem não muda.'),
      campo({ name: 'os_id', label: 'Ordem de serviço *', type: 'select', required: true,
        options: abertas.map((o) => [o.id, `OS nº ${o.id} — ${o.veiculo_placa} (${o.cliente_nome || 'sem cliente'})`]) }),
      seletor.node,
      campo({ name: 'observacoes', label: 'Motivo (ex.: pastilha de freio gasta)', type: 'textarea' }, ''),
      erroArea,
      el('div', { class: 'acoes' },
        el('button', { type: 'submit', class: 'primario' }, 'Criar adicional'),
        el('button', { type: 'button', class: 'link', onclick: () => formArea.replaceChildren() }, 'cancelar')),
    );
    formArea.replaceChildren(el('h3', {}, 'Adicional durante o serviço'), form);
  }

  async function carregarLista() {
    try {
      const orcs = await chamar('orcamento_listar');
      const linhas = orcs.map((o) => ({
        ...o,
        numero: o.tipo === 'ADITIVO' ? `Adicional da OS nº ${o.ordem_id}` : `Orçamento nº ${o.id}`,
        carro: o.veiculo_placa ? `${o.veiculo_placa} — ${o.veiculo_modelo}` : '',
        situacao_txt: el('span', { class: 'status' }, ROTULOS[o.status] ?? o.status),
        total_txt: centavosParaMoeda(o.valor_total_centavos),
        __acoes: el('button', { class: 'link', onclick: () => mostrarDetalhe(o.id) }, 'abrir'),
      }));
      listaArea.replaceChildren(
        el('h3', {}, 'Orçamentos e adicionais'),
        el('div', { class: 'acoes' },
          botao({ texto: 'Novo orçamento', icone: 'mais', tipo: 'primario', onclick: mostrarNovo }),
          botao({ texto: 'Adicional em uma OS', icone: 'ordens', onclick: mostrarAdicional })),
        tabela([
          { key: 'numero', label: 'Nº' },
          { key: 'cliente_nome', label: 'Cliente' },
          { key: 'carro', label: 'Carro' },
          { key: 'situacao_txt', label: 'Situação', render: (l) => l.situacao_txt },
          { key: 'total_txt', label: 'Total' },
        ], linhas, 'Nenhum orçamento ainda.'),
      );
    } catch (e) {
      aviso(e.texto, 'erro');
    }
  }

  // Forma de aprovação: a oficina registra como o cliente aprovou. Não é um documento que o cliente assina.
  function formAprovacao(o) {
    const erroArea = el('p', { class: 'erro-campo', role: 'alert' });
    const form = el('form', {
      onsubmit: async (ev) => {
        ev.preventDefault();
        const d = Object.fromEntries(new FormData(ev.target).entries());
        erroArea.textContent = '';
        try {
          await chamar('orcamento_aprovar', { id: o.id, forma: d.forma, observacao: d.observacao || null });
          aviso('Aprovação registrada.');
          await carregarLista();
          mostrarDetalhe(o.id);
        } catch (e) {
          erroArea.textContent = Object.values(e.campos ?? {}).join(' ') || e.texto;
        }
      },
    },
      el('p', { class: 'ajuda' }, 'Registre como o cliente aprovou. Quem registra é a oficina, com base no que o cliente informou.'),
      campo({ name: 'forma', label: 'Como o cliente aprovou? *', type: 'select', required: true, options: FORMAS }, ''),
      campo({ name: 'observacao', label: 'Observação (obrigatória se for "Outra forma")', type: 'textarea' }, ''),
      erroArea,
      el('div', { class: 'acoes' },
        el('button', { type: 'submit', class: 'primario' }, 'Registrar aprovação'),
        el('button', { type: 'button', class: 'link', onclick: () => mostrarDetalhe(o.id) }, 'cancelar')));
    return form;
  }

  function botoesDeAcao(o) {
    const acao = (texto, tipo, fn) => botao({ texto, tipo, onclick: async () => {
      try { await fn(); } catch (e) { aviso(e.texto, 'erro'); }
    } });
    const recarregar = async (msg) => { if (msg) aviso(msg); await carregarLista(); mostrarDetalhe(o.id); };
    const itens = [];

    if (o.status === 'RASCUNHO') {
      itens.push(acao('Marcar como enviado', 'primario', async () => {
        await chamar('orcamento_enviar', { id: o.id });
        await recarregar('Orçamento marcado como enviado. Agora aguarde o cliente aprovar.');
      }));
    }
    if (o.status === 'AGUARDANDO_APROVACAO') {
      itens.push(botao({ texto: 'Aprovado pelo cliente', icone: 'ok', tipo: 'primario', onclick: () => {
        detalheArea.querySelector('.registrar-aprovacao')?.remove();
        detalheArea.append(el('div', { class: 'registrar-aprovacao' }, el('h4', {}, 'Registrar a aprovação'), formAprovacao(o)));
      } }));
      itens.push(acao('Recusado', 'perigo', async () => {
        if (!confirmar('O cliente recusou este orçamento? Ele não poderá mais virar ordem de serviço.')) return;
        await chamar('orcamento_recusar', { id: o.id });
        await recarregar('Orçamento marcado como recusado.');
      }));
    }
    if (['RASCUNHO', 'AGUARDANDO_APROVACAO'].includes(o.status)) {
      itens.push(acao('Cancelar orçamento', 'perigo', async () => {
        if (!confirmar('Cancelar este orçamento? Ele continua guardado, mas não pode mais ser aprovado.')) return;
        await chamar('orcamento_cancelar', { id: o.id });
        await recarregar('Orçamento cancelado.');
      }));
    }
    itens.push(acao('Imprimir orçamento', 'secundario', async () => {
      await invocar('orcamento_imprimir', { id: o.id });
      aviso('Orçamento enviado para a impressora. Ele não é nota fiscal.');
    }));
    return itens;
  }

  async function mostrarDetalhe(id) {
    try {
      const o = await chamar('orcamento_buscar', { id });
      const titulo = o.tipo === 'ADITIVO' ? `Adicional da OS nº ${o.ordem_id}` : `Orçamento nº ${o.id}`;
      const linhasTab = o.itens.map((l) => ({
        ...l,
        tipo_txt: l.tipo === 'servico' ? 'Serviço' : 'Peça',
        unit: centavosParaMoeda(l.preco_unitario_centavos),
        sub: centavosParaMoeda(l.subtotal_centavos),
      }));

      let blocoAprovacao = null;
      if (o.status === 'APROVADO') {
        blocoAprovacao = el('div', { class: 'faixa' },
          icone('ok', 22),
          el('p', {}, `Aprovação registrada pela oficina em ${dataHora(o.aprovado_em)}, informada pelo cliente: ${ROTULO_FORMA[o.aprovacao_forma] ?? o.aprovacao_forma ?? '—'}.`),
          o.aprovacao_obs ? el('p', {}, `Observação: ${o.aprovacao_obs}`) : null);
      }

      let blocoOs = null;
      if (o.status === 'APROVADO' && o.tipo === 'ORCAMENTO' && !o.ordem_id) {
        blocoOs = el('div', { class: 'faixa alerta' },
          el('p', {}, 'Ao transformar em ordem de serviço, este orçamento NÃO é apagado. Ele continua guardado aqui, com os valores que foram aprovados.'),
          botao({ texto: 'Transformar em ordem de serviço', icone: 'ordens', tipo: 'primario', onclick: async () => {
            try {
              const os = await chamar('orcamento_converter_os', { id: o.id });
              aviso(`Ordem de serviço nº ${os.id} criada. O orçamento continua guardado aqui.`);
              await carregarLista();
              mostrarDetalhe(o.id);
            } catch (e) { aviso(e.texto, 'erro'); }
          } }));
      } else if (o.ordem_id && o.tipo === 'ORCAMENTO') {
        blocoOs = el('p', { class: 'faixa' }, `Este orçamento já virou a ordem de serviço nº ${o.ordem_id}. Acompanhe pela tela Carros na Oficina.`);
      } else if (o.tipo === 'ADITIVO' && o.status === 'APROVADO') {
        blocoOs = el('p', { class: 'faixa' }, `Este adicional já foi somado à ordem de serviço nº ${o.ordem_id}.`);
      }

      detalheArea.replaceChildren(
        el('h3', {}, titulo),
        el('p', {}, el('strong', {}, `Situação: ${ROTULOS[o.status] ?? o.status}`)),
        el('p', {}, `Cliente: ${o.cliente_nome} · Carro: ${o.veiculo_placa} — ${o.veiculo_modelo} · Responsável: ${o.funcionario_nome}`),
        o.validade ? el('p', {}, `Valores válidos até ${dataBr(o.validade)}.`) : null,
        o.prazo_dias != null ? el('p', {}, `Prazo estimado do serviço: ${o.prazo_dias} dias.`) : null,
        o.observacoes ? el('p', {}, `Observações: ${o.observacoes}`) : null,
        blocoAprovacao,
        tabela([
          { key: 'nome', label: 'Peça ou serviço' },
          { key: 'tipo_txt', label: 'Tipo' },
          { key: 'quantidade', label: 'Qtd' },
          { key: 'unit', label: 'Preço unit.' },
          { key: 'sub', label: 'Subtotal' },
        ], linhasTab, 'Sem itens.'),
        el('p', {}, el('strong', {}, `Total: ${centavosParaMoeda(o.valor_total_centavos)}`)),
        blocoOs,
        el('div', { class: 'acoes' }, botoesDeAcao(o)),
      );
    } catch (e) {
      aviso(e.texto, 'erro');
    }
  }

  await carregarLista();
}

