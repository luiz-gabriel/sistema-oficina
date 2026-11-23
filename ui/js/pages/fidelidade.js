// Fidelidade: saldo de pontos e extrato de cada cliente. 1 ponto a cada R$ 10,00 pagos em OS ou venda de balcão.
// Pode abrir já com um cliente (ctx.opcoes.cliente_id), por exemplo a partir da tela de Clientes.
import { chamar } from '../api.js';
import { el, aviso, botao, cabecalhoPagina, icone, moeda, tabela } from '../components.js';

const MAX_RESULTADOS = 8;
const ROTULO_ORIGEM = { OS: 'Ordem de serviço', VENDA: 'Venda rápida', RESGATE: 'Resgate' };
const ROTULO_REF = { OS: 'OS', VENDA: 'Venda', RESGATE: '' };
const dataHora = (texto) => (texto ? new Date(texto).toLocaleString('pt-BR') : '');

// 100 pontos = R$ 10,00 de desconto. Por enquanto o saldo só é consultado; o uso no caixa vem na próxima etapa.
const descontoDisponivel = (saldo) => moeda(Math.floor(saldo / 100) * 10);

export async function render(container, ctx = {}) {
  const clientes = await chamar('clientes_listar');
  const buscaCliente = el('input', { type: 'search', placeholder: 'Nome ou telefone', 'aria-label': 'Buscar cliente', autocomplete: 'off' });
  const resultados = el('div', { class: 'resultados-busca' });
  const detalhe = el('div', {});

  function filtrar(texto) {
    const t = texto.trim().toLowerCase();
    if (!t) return [];
    return clientes
      .filter((c) => c.nome.toLowerCase().includes(t) || (c.telefone ?? '').toLowerCase().includes(t))
      .slice(0, MAX_RESULTADOS);
  }

  buscaCliente.addEventListener('input', () => {
    resultados.replaceChildren(...filtrar(buscaCliente.value).map((c) => el('div', { class: 'linha-resultado' },
      el('span', {}, c.nome),
      c.telefone ? el('small', { class: 'ajuda' }, ` ${c.telefone}`) : null,
      botao({ texto: 'Ver pontos', icone: 'seta', onclick: () => mostrar(c) }))));
  });

  async function mostrar(cliente) {
    detalhe.replaceChildren(el('p', { class: 'ajuda' }, 'Carregando...'));
    try {
      const [situacao, extrato] = await Promise.all([
        chamar('fidelidade_saldo', { cliente_id: cliente.id }),
        chamar('fidelidade_extrato', { cliente_id: cliente.id }),
      ]);
      const linhas = extrato.map((m) => ({
        data: dataHora(m.criado_em),
        origem: ROTULO_ORIGEM[m.origem] ?? m.origem,
        referencia: m.referencia ? `${ROTULO_REF[m.origem] ?? ''} #${m.referencia}`.trim() : '—',
        pontos: m.pontos > 0 ? `+${m.pontos}` : String(m.pontos),
      }));
      detalhe.replaceChildren(el('section', { class: 'card' },
        el('h3', {}, icone('clientes', 24), cliente.nome),
        el('div', { class: 'indicadores-caixa' },
          el('div', { class: 'indicador-caixa destaque' },
            el('span', { class: 'rotulo' }, 'Saldo de pontos'),
            el('strong', {}, `${situacao.saldo} pontos`)),
          el('div', { class: 'indicador-caixa' },
            el('span', { class: 'rotulo' }, 'Desconto disponível'),
            el('strong', {}, descontoDisponivel(situacao.saldo)))),
        el('p', { class: 'ajuda' }, 'Resgate: 100 pontos = R$ 10,00 de desconto. O uso dos pontos no caixa ainda não está disponível.'),
        el('h4', {}, 'Extrato'),
        tabela([
          { key: 'data', label: 'Data' },
          { key: 'origem', label: 'Origem' },
          { key: 'referencia', label: 'Referência' },
          { key: 'pontos', label: 'Pontos' },
        ], linhas, 'Este cliente ainda não tem pontos.')));
    } catch (e) {
      detalhe.replaceChildren();
      aviso(e.texto ?? String(e), 'erro');
    }
  }

  container.replaceChildren(
    cabecalhoPagina('Fidelidade', 'clientes', 'Pontos de cada cliente: 1 ponto a cada R$ 10,00 pagos em ordens de serviço e vendas.'),
    el('div', { class: 'card' },
      el('label', { class: 'campo' }, el('span', { class: 'rotulo' }, 'Buscar cliente'), buscaCliente),
      resultados),
    detalhe,
  );

  const inicial = ctx.opcoes?.cliente_id != null ? clientes.find((c) => c.id === Number(ctx.opcoes.cliente_id)) : null;
  if (inicial) await mostrar(inicial);
  else buscaCliente.focus();
}
