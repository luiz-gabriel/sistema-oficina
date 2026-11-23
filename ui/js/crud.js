// Converte valores do formulário (texto) para os tipos que o Rust espera.
// Vazio vira null; campos numéricos e chaves *_id viram número.
export function coerceDados(campos, bruto) {
  const dados = {};
  for (const c of campos) {
    const v = bruto[c.name];
    if (v === '' || v === undefined) dados[c.name] = null;
    else if (c.type === 'number' || c.name.endsWith('_id')) dados[c.name] = Number(v);
    else dados[c.name] = v;
  }
  return dados;
}

// Página CRUD genérica: lista + formulário. Clientes, veículos, itens e funcionários usam esta base.
import { chamar } from './api.js';
import { el, aviso, campo, tabela, confirmar, botao, icone, cabecalhoPagina } from './components.js';

// Tamanho de cada bloco da lista. Listas com `paginada: true` carregam o próximo bloco quando a pessoa
// chega perto do fim da tabela (rolagem contínua), em vez de trazer o cadastro inteiro de uma vez.
export const LOTE_LISTA = 80;

// config = { titulo, icone, descricao, comando, colunas, campos, vazio, paginada?, topo?(elemento, api), extraEdicao?(registro),
//            acoesLinha?(registro, ctx) => nó[] }
// Campos podem usar `options` como função assíncrona para listas de outros cadastros.
// Ao abrir a página, `ctx.opcoes.valores` (se houver) preenche o formulário de cadastro novo
// (ex.: { cliente_id } vindo da tela de clientes).
export function crudPage(config) {
  return async function render(container, ctx = {}) {
    let editando = null;
    let rascunho = ctx.opcoes?.valores ?? null;
    let erros = {};
    let opcoes = {};
    // Bloco extra exibido só no cadastro novo (ex.: fotos do carro). Criado a cada formulário novo.
    let novoExtra = null;

    for (const c of config.campos) {
      if (typeof c.options === 'function') opcoes[c.name] = await c.options();
    }

    const formArea = el('section', { class: 'card' });
    const listaArea = el('section', { class: 'card' });
    const topo = el('div', {});
    container.replaceChildren(
      cabecalhoPagina(config.titulo, config.icone, config.descricao),
      topo, formArea, listaArea,
    );

    const api = {
      preencher(valores) {
        editando = null;
        rascunho = valores;
        erros = {};
        desenharForm();
        formArea.scrollIntoView({ behavior: 'smooth', block: 'start' });
      },
      novo() {
        editando = null;
        rascunho = null;
        erros = {};
        novoExtra = null;
        desenharForm();
      },
    };
    if (config.topo) config.topo(topo, api);
    // Atalho de cadastro (ex.: Ctrl+F4 em Peças): abre direto o formulário de cadastro novo.
    if (ctx.opcoes?.novo) api.novo();

    // Estado da lista: registros já carregados, se acabou, e uma "versão" para descartar respostas atrasadas
    // (se a lista for recarregada enquanto um bloco ainda vem, o bloco antigo não entra na lista nova).
    let carregados = [];
    let fimDaLista = false;
    let carregandoBloco = false;
    let versaoLista = 0;
    let observadorFim = null;

    async function carregarLista() {
      versaoLista += 1;
      carregados = [];
      fimDaLista = false;
      carregandoBloco = false;
      await carregarBloco();
    }

    async function carregarBloco() {
      if (carregandoBloco || fimDaLista) return;
      const versao = versaoLista;
      carregandoBloco = true;
      try {
        const bloco = config.paginada
          ? await chamar(`${config.comando}_pagina`, { limite: LOTE_LISTA, deslocamento: carregados.length })
          : await chamar(`${config.comando}_listar`);
        if (versao !== versaoLista) return;
        carregados = carregados.concat(bloco);
        fimDaLista = !config.paginada || bloco.length < LOTE_LISTA;
        desenharLista();
      } catch (e) {
        if (versao === versaoLista) aviso(e.texto, 'erro');
      } finally {
        if (versao === versaoLista) carregandoBloco = false;
      }
    }

    function linhaComAcoes(linha) {
      return {
        ...linha,
        __acoes: el('span', { class: 'botoes-linha' },
          botao({ texto: 'Editar', icone: 'lapis', tipo: 'link', onclick: () => { editando = linha; erros = {}; desenharForm(); formArea.scrollIntoView({ behavior: 'smooth' }); } }),
          botao({ texto: 'Excluir', icone: 'lixeira', tipo: 'link perigo', onclick: () => remover(linha) }),
          ...(config.acoesLinha?.(linha, ctx) ?? []),
        ),
      };
    }

    // Desenha os blocos carregados. Sem paginação, a lista já é completa e não há o que buscar.
    function desenharLista() {
      observadorFim?.disconnect();
      observadorFim = null;
      const rodape = config.paginada && !fimDaLista && carregados.length
        ? el('div', { class: 'carregar-mais' },
            el('small', { class: 'ajuda' }, `${carregados.length} carregados. Role para ver mais.`),
            botao({ texto: 'Carregar mais', icone: 'seta', onclick: () => carregarBloco() }))
        : (config.paginada && carregados.length ? el('small', { class: 'ajuda carregar-mais' }, 'Fim da lista.') : null);
      listaArea.replaceChildren(
        el('h3', {}, 'Cadastrados'),
        tabela(config.colunas, carregados.map(linhaComAcoes), config.vazio),
        rodape,
      );
      if (rodape && 'IntersectionObserver' in window) {
        observadorFim = new IntersectionObserver((entradas) => {
          if (entradas.some((e) => e.isIntersecting)) carregarBloco();
        }, { rootMargin: '240px' });
        observadorFim.observe(rodape);
      }
    }

    function desenharForm() {
      const valores = editando ?? rascunho ?? {};
      const form = el('form', {
        onsubmit: async (ev) => {
          ev.preventDefault();
          const dados = coerceDados(config.campos, Object.fromEntries(new FormData(ev.target).entries()));
          try {
            if (editando) {
              await chamar(`${config.comando}_atualizar`, { id: editando.id, dados });
              aviso('Alterações salvas.');
            } else {
              const criado = await chamar(`${config.comando}_criar`, { dados });
              aviso('Cadastro salvo com sucesso.');
              if (novoExtra?.enviar) {
                try { await novoExtra.enviar(criado); } catch (e2) { aviso(`Cadastro salvo, mas uma foto não foi enviada: ${e2.texto ?? e2}`, 'erro'); }
              }
            }
            editando = null;
            rascunho = null;
            erros = {};
            novoExtra = null;
            desenharForm();
            carregarLista();
          } catch (e) {
            erros = e.campos;
            aviso(e.texto, 'erro');
            desenharForm();
          }
        },
      },
        config.campos.map((c) => campo({ ...c, options: typeof c.options === 'function' ? opcoes[c.name] : c.options }, valores[c.name], erros[c.name])),
        el('div', { class: 'acoes' },
          botao({ texto: editando ? 'Salvar alterações' : 'Cadastrar', icone: editando ? 'ok' : 'mais', tipo: 'primario', type: 'submit' }),
          editando || rascunho ? botao({ texto: 'Cancelar', icone: 'x', onclick: () => api.novo() }) : null,
        ),
      );
      const titulo = editando ? `Alterar ${config.singular ?? 'cadastro'}` : `Novo ${config.singular ?? 'cadastro'}`;
      if (!editando && config.extraNovo && !novoExtra) novoExtra = config.extraNovo();
      formArea.replaceChildren(el('h3', {}, icone(editando ? 'lapis' : 'mais', 22), titulo), form,
        editando && config.extraEdicao ? config.extraEdicao(editando) : null,
        !editando && novoExtra ? novoExtra.node : null);
      formArea.dataset.modo = editando ? 'editar' : 'novo';
    }

    async function remover(linha) {
      if (!confirmar('Tem certeza que deseja excluir este cadastro?')) return;
      try {
        await chamar(`${config.comando}_remover`, { id: linha.id });
        aviso('Cadastro excluído.');
        carregarLista();
      } catch (e) {
        aviso(e.texto, 'erro');
      }
    }

    desenharForm();
    carregarLista();
  };
}
