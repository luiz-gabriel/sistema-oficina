// Ordens de serviço: criação com itens, mudança de status e exclusão.
import { chamar, invocar } from '../api.js';
import { el, aviso, botao, campo, icone, moeda, tabela, confirmar, cabecalhoPagina } from '../components.js';

const ROTULOS_STATUS = {
  CRIADA: 'Criada',
  EM_ANDAMENTO: 'Em andamento',
  AGUARDANDO_PAGAMENTO: 'Aguardando pagamento',
  CONCLUIDA: 'Concluída',
  CANCELADA: 'Cancelada',
};

// Próximos status permitidos (espelha a máquina de estados do backend).
const PROXIMOS = {
  CRIADA: ['EM_ANDAMENTO', 'CANCELADA'],
  EM_ANDAMENTO: ['AGUARDANDO_PAGAMENTO', 'CANCELADA'],
  AGUARDANDO_PAGAMENTO: ['CONCLUIDA', 'CANCELADA'],
  CONCLUIDA: [],
  CANCELADA: [],
};

export async function render(container, ctx = {}) {
  const [veiculos, funcionarios, itens] = await Promise.all([
    chamar('veiculos_listar'),
    chamar('funcionarios_listar'),
    chamar('itens_listar'),
  ]);
  const clientes = await chamar('clientes_listar');
  const nomeCliente = Object.fromEntries(clientes.map((c) => [c.id, c.nome]));
  // Receber pagamento exige caixa aberto: avisa logo na entrada da tela.
  const caixaSit = await chamar('caixa_situacao');
  const avisoCaixa = caixaSit.aberto ? null : el('div', { class: 'faixa alerta', role: 'alert' },
    icone('alerta', 26),
    el('p', {}, 'O caixa está fechado. Para receber o pagamento de uma ordem, abra o caixa antes.'),
    botao({ texto: 'Abrir o caixa', icone: 'caixa', tipo: 'primario', onclick: () => ctx.irPara?.('caixa') }));

  const formArea = el('div', { class: 'card' });
  const listaArea = el('div', { class: 'card' });
  const detalheArea = el('div', { class: 'card' });
  container.replaceChildren(cabecalhoPagina('Ordens de Serviço', 'ordens', 'Abra uma ordem para cada carro que entrar na oficina.'), avisoCaixa, formArea, listaArea, detalheArea);
  if (ctx.opcoes?.novo) formArea.scrollIntoView({ block: 'start' });

  const opcoesVeiculo = veiculos.map((v) => [v.id, `${v.placa} — ${v.modelo} (${nomeCliente[v.cliente_id] ?? '?'})`]);
  const opcoesFuncionario = funcionarios.map((f) => [f.id, f.nome]);
  const opcoesItem = itens.map((i) => [i.id, `${i.nome} — ${moeda(i.preco_venda)}`]);

  // Linhas de itens do formulário de criação (adicionadas/removidas dinamicamente).
  let linhas = [];
  // Cabeçalho (veículo, responsável, descrição) guardado entre re-renderizações do formulário.
  let cabecalho = { veiculo_id: '', funcionario_id: '', descricao: '' };

  function desenharFormulario() {
    const listaItens = el('div', { class: 'itens-os' },
      linhas.map((linha, idx) => el('div', { class: 'linha-item' },
        campo({ name: `item_${idx}`, label: 'Item', type: 'select', options: opcoesItem }, linha.item_id ?? ''),
        campo({ name: `qtd_${idx}`, label: 'Quantidade', type: 'number', min: 1 }, linha.quantidade ?? 1),
        el('button', { type: 'button', class: 'link', onclick: () => { lerLinhas(form); linhas.splice(idx, 1); desenharFormulario(); } }, 'remover'),
      )),
    );

    const form = el('form', {
      onsubmit: async (ev) => {
        ev.preventDefault();
        const dados = Object.fromEntries(new FormData(ev.target).entries());
        try {
          await chamar('ordens_criar', {
            dados: {
              veiculo_id: Number(dados.veiculo_id),
              funcionario_id: Number(dados.funcionario_id),
              descricao: dados.descricao || null,
              itens: lerLinhas(ev.target),
            },
          });
          aviso('Ordem de serviço criada.');
          linhas = [];
          cabecalho = { veiculo_id: '', funcionario_id: '', descricao: '' };
          desenharFormulario();
          carregarLista();
        } catch (e) {
          aviso(e.texto, 'erro');
        }
      },
    },
      campo({ name: 'veiculo_id', label: 'Veículo *', type: 'select', required: true, options: opcoesVeiculo }, cabecalho.veiculo_id),
      campo({ name: 'funcionario_id', label: 'Responsável *', type: 'select', required: true, options: opcoesFuncionario }, cabecalho.funcionario_id),
      campo({ name: 'descricao', label: 'Descrição do problema', type: 'textarea' }, cabecalho.descricao),
      el('div', { class: 'acoes' },
        el('button', { type: 'button', class: 'secundario', onclick: () => { lerLinhas(form); linhas.push({ quantidade: 1 }); desenharFormulario(); } }, '+ item'),
        el('button', { type: 'submit', class: 'primario' }, 'Abrir ordem'),
      ),
      listaItens,
    );
    formArea.replaceChildren(el('h3', {}, 'Nova ordem de serviço'), leitorCodigo(), form);
  }

  // Leitor de código de barras: funciona como teclado (digita o código e manda Enter).
  // Fica fora do formulário para o Enter do leitor não enviar a ordem.
  function leitorCodigo() {
    const leitor = el('input', { id: 'leitor-codigo', autocomplete: 'off', placeholder: 'Bipe o código de barras da peça (ou digite e tecle Enter)', 'aria-label': 'Leitor de código de barras' });
    leitor.addEventListener('keydown', async (ev) => {
      if (ev.key !== 'Enter') return;
      ev.preventDefault();
      const codigo = leitor.value.trim();
      leitor.value = '';
      if (!codigo) return;
      try {
        const item = await chamar('itens_buscar_codigo', { codigo });
        const atuais = lerLinhas(formArea.querySelector('form'));
        const existente = atuais.find((l) => l.item_id === item.id);
        if (existente) existente.quantidade = (existente.quantidade ?? 1) + 1;
        else atuais.push({ item_id: item.id, quantidade: 1 });
        linhas = atuais;
        desenharFormulario();
        formArea.querySelector('#leitor-codigo')?.focus();
        aviso(`${item.nome} adicionado.`);
      } catch (e) {
        aviso(e.texto, 'erro');
        leitor.focus();
      }
    });
    return el('div', { class: 'leitor-linha' }, leitor);
  }

  // Lê as linhas e o cabeçalho digitados e guarda em `linhas`/`cabecalho` (antes de re-renderizar).
  function lerLinhas(form) {
    const v = Object.fromEntries(new FormData(form).entries());
    cabecalho = {
      veiculo_id: v.veiculo_id ?? '',
      funcionario_id: v.funcionario_id ?? '',
      descricao: v.descricao ?? '',
    };
    linhas = linhas.map((_, idx) => ({
      item_id: v[`item_${idx}`] ? Number(v[`item_${idx}`]) : null,
      quantidade: v[`qtd_${idx}`] ? Number(v[`qtd_${idx}`]) : null,
    }));
    return linhas;
  }

  async function carregarLista() {
    try {
      const ordens = await chamar('ordens_listar');
      const comAcoes = ordens.map((o) => ({
        ...o,
        // Venda de balcão não tem carro: mostra o tipo no lugar da placa.
        veiculo_placa: o.tipo === 'BALCAO' ? 'Venda de balcão' : (o.veiculo_placa ?? ''),
        cliente_nome: o.cliente_nome ?? (o.tipo === 'BALCAO' ? '—' : ''),
        status_texto: el('span', { class: 'status' }, ROTULOS_STATUS[o.status] ?? o.status),
        valor_fmt: moeda(o.valor_total),
        __acoes: el('button', { class: 'link', onclick: () => mostrarDetalhe(o.id) }, 'abrir'),
      }));
      listaArea.replaceChildren(
        el('h3', {}, 'Ordens'),
        tabela([
          { key: 'id', label: 'Nº' },
          { key: 'veiculo_placa', label: 'Placa' },
          { key: 'cliente_nome', label: 'Cliente' },
          { key: 'status_texto', label: 'Status', render: (o) => o.status_texto },
          { key: 'valor_fmt', label: 'Total', render: (o) => o.valor_fmt },
        ], comAcoes, 'Nenhuma ordem de serviço.'),
      );
    } catch (e) {
      aviso(e.texto, 'erro');
    }
  }

  // Concluir é receber: pergunta a forma de pagamento antes de fechar a ordem.
  // Se o caixa estiver fechado, não mostra as formas: oferece abrir o caixa (o backend recusaria a conclusão).
  async function escolherPagamento(id) {
    detalheArea.querySelector('.pagamento')?.remove();
    const caixaSit = await chamar('caixa_situacao');
    const formas = [['dinheiro', 'Dinheiro'], ['pix', 'PIX'], ['cartao', 'Cartão'], ['outro', 'Outra forma']];
    const bloco = caixaSit.aberto
      ? el('div', { class: 'pagamento' },
        el('h4', {}, 'Como o cliente vai pagar?'),
        el('p', { class: 'ajuda' }, 'O valor entra no caixa de hoje na forma escolhida.'),
        el('div', { class: 'acoes' }, formas.map(([valor, texto]) => botao({ texto, icone: 'ok', tipo: 'primario', onclick: async () => {
          try {
            await chamar('ordens_alterar_status', { id, status: 'CONCLUIDA', pagamento: valor });
            aviso('Ordem concluída e pagamento registrado.');
            await carregarLista();
            mostrarDetalhe(id);
          } catch (e) { aviso(e.texto, 'erro'); }
        } }))))
      : el('div', { class: 'pagamento faixa alerta', role: 'alert' },
        icone('alerta', 26),
        el('p', {}, 'Para concluir e receber o pagamento, o caixa precisa estar aberto.'),
        botao({ texto: 'Abrir o caixa', icone: 'caixa', tipo: 'primario', onclick: () => ctx.irPara?.('caixa') }));
    detalheArea.append(bloco);
  }

  async function mostrarDetalhe(id) {
    try {
      const o = await chamar('ordens_buscar', { id });
      const botoes = PROXIMOS[o.status].map((st) => el('button', {
        class: st === 'CANCELADA' ? 'perigo' : 'primario',
        onclick: async () => {
          // Concluir o serviço e receber são passos separados: o recebimento é cobrado na etapa seguinte.
          if (st === 'AGUARDANDO_PAGAMENTO') {
            try {
              await chamar('os_concluir_servico', { id });
              aviso('Serviço concluído. Agora registre o pagamento.');
              await carregarLista();
              mostrarDetalhe(id);
            } catch (e) { aviso(e.texto, 'erro'); }
            return;
          }
          if (st === 'CONCLUIDA') return escolherPagamento(id);
          if (st === 'CANCELADA' && !confirmar('Cancelar esta ordem?')) return;
          try {
            await chamar('ordens_alterar_status', { id, status: st });
            aviso(`Status: ${ROTULOS_STATUS[st]}.`);
            await carregarLista();
            mostrarDetalhe(id);
          } catch (e) {
            aviso(e.texto, 'erro');
          }
        },
      }, `Mover para ${ROTULOS_STATUS[st]}`));

      detalheArea.replaceChildren(
        el('h3', {}, `OS #${o.id} — ${o.veiculo_placa} (${o.veiculo_modelo})`),
        el('p', {}, `Cliente: ${o.cliente_nome} · Responsável: ${o.funcionario_nome}`),
        el('p', {}, `Status: ${ROTULOS_STATUS[o.status]} · Entrada: ${new Date(o.data_entrada).toLocaleString('pt-BR')}`),
        o.descricao ? el('p', {}, `Problema: ${o.descricao}`) : null,
        tabela([
          { key: 'item_nome', label: 'Item' },
          { key: 'tipo', label: 'Tipo', render: (l) => (l.tipo === 'peca' ? 'Peça' : 'Serviço') },
          { key: 'quantidade', label: 'Qtd' },
          { key: 'preco_unitario', label: 'Unit.', render: (l) => moeda(l.preco_unitario) },
          { key: 'subtotal', label: 'Subtotal', render: (l) => moeda(l.subtotal) },
        ], o.itens, 'Sem itens nesta ordem.'),
        el('p', {}, el('strong', {}, `Total: ${moeda(o.valor_total)}`)),
        el('div', { class: 'acoes' }, botoes,
          el('button', { class: 'secundario', onclick: async () => {
            try { await invocar('ordens_imprimir', { id }); aviso('Cupom enviado para a impressora.'); }
            catch (e) { aviso(e.texto, 'erro'); }
          } }, 'Imprimir cupom'),
          o.status === 'CRIADA'
            ? el('button', { class: 'secundario', onclick: () => editarItens(o) }, 'Editar itens')
            : null,
          ['CRIADA', 'CANCELADA'].includes(o.status)
            ? el('button', { class: 'secundario', onclick: async () => {
                if (!confirmar('Excluir esta ordem?')) return;
                try { await chamar('ordens_excluir', { id }); aviso('Ordem excluída.'); detalheArea.replaceChildren(); carregarLista(); }
                catch (e) { aviso(e.texto, 'erro'); }
              } }, 'Excluir')
            : null),
      );
    } catch (e) {
      aviso(e.texto, 'erro');
    }
  }

  // Editor de itens de uma OS já criada (só permitido enquanto está CRIADA).
  function editarItens(o) {
    let itensEdit = o.itens.map((l) => ({ item_id: l.item_id, quantidade: l.quantidade }));
    const erroArea = el('p', { class: 'erro-campo' });

    function lerEdicao(form) {
      const v = Object.fromEntries(new FormData(form).entries());
      itensEdit = itensEdit.map((_, idx) => ({
        item_id: v[`eitem_${idx}`] ? Number(v[`eitem_${idx}`]) : null,
        quantidade: v[`eqtd_${idx}`] ? Number(v[`eqtd_${idx}`]) : null,
      }));
      return itensEdit;
    }

    function desenhar() {
      const form = el('form', {
        onsubmit: async (ev) => {
          ev.preventDefault();
          try {
            await chamar('ordens_substituir_itens', { id: o.id, itens: lerEdicao(ev.target) });
            aviso('Itens atualizados.');
            mostrarDetalhe(o.id);
            carregarLista();
          } catch (e) {
            erroArea.textContent = e.texto;
          }
        },
      },
        itensEdit.map((linha, idx) => el('div', { class: 'linha-item' },
          campo({ name: `eitem_${idx}`, label: 'Item', type: 'select', options: opcoesItem }, linha.item_id ?? ''),
          campo({ name: `eqtd_${idx}`, label: 'Quantidade', type: 'number', min: 1 }, linha.quantidade ?? 1),
          el('button', { type: 'button', class: 'link', onclick: () => { lerEdicao(form); itensEdit.splice(idx, 1); desenhar(); } }, 'remover'),
        )),
        el('div', { class: 'acoes' },
          el('button', { type: 'button', class: 'secundario', onclick: () => { lerEdicao(form); itensEdit.push({ quantidade: 1 }); desenhar(); } }, '+ item'),
          el('button', { type: 'submit', class: 'primario' }, 'Salvar itens'),
          el('button', { type: 'button', class: 'link', onclick: () => mostrarDetalhe(o.id) }, 'cancelar'),
        ),
        erroArea,
      );
      detalheArea.replaceChildren(el('h3', {}, `Editar itens da OS #${o.id}`), form);
    }
    desenhar();
  }

  desenharFormulario();
  carregarLista();
}
