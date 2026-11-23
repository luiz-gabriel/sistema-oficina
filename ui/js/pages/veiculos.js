// Veículos: busca no catálogo (marca, modelo, ano, motor), cadastro e fotos do carro.
import { crudPage } from '../crud.js';
import { chamar } from '../api.js';
import { el, aviso, botao, icone, confirmar, moeda } from '../components.js';

const CORES = ['Branco', 'Preto', 'Prata', 'Cinza', 'Vermelho', 'Azul', 'Verde', 'Amarelo', 'Laranja', 'Marrom', 'Bege', 'Dourado', 'Vinho'];

const clientesOpcoes = async () => (await chamar('clientes_listar')).map((c) => [c.id, c.nome]);
// Responsável é opcional: a primeira opção (vazia) significa "sem responsável".
const responsaveisOpcoes = async () => [['', '— sem responsável —'], ...(await chamar('funcionarios_listar')).map((f) => [f.id, f.nome])];

const LIMITE_RESULTADOS = 30;
const TAMANHO_MAXIMO = 5 * 1024 * 1024;
let catalogo = null;
async function carregarCatalogo() {
  if (!catalogo) {
    const res = await fetch('data/veiculos-brasileiros.json');
    if (!res.ok) throw new Error('Catálogo de veículos não pôde ser carregado.');
    catalogo = await res.json();
  }
  return catalogo;
}

const normalizar = (t) => String(t).normalize('NFD').replace(/[̀-ͯ]/g, '').toLowerCase().trim();
export function buscarNoCatalogo(lista, termo) {
  const palavras = normalizar(termo).split(/\s+/).filter(Boolean);
  if (!palavras.length) return [];
  return lista.filter((m) => {
    const texto = normalizar(`${m.marca} ${m.modelo} ${m.ano ?? ''}`);
    return palavras.every((p) => texto.includes(p));
  });
}

function buscaVeiculo(topo, api) {
  const resultados = el('div', { class: 'resultados-busca', 'aria-live': 'polite' });
  const campoBusca = el('input', { type: 'search', placeholder: 'Digite o nome do carro. Ex.: Gol, Civic, Uno 2015…', 'aria-label': 'Buscar carro' });

  async function atualizar() {
    const termo = campoBusca.value;
    let lista;
    try { lista = await carregarCatalogo(); } catch (e) { resultados.replaceChildren(el('p', { class: 'erro-campo' }, e.message)); return; }
    if (!normalizar(termo)) { resultados.replaceChildren(); return; }
    const achados = buscarNoCatalogo(lista, termo);
    if (achados.length) {
      const mostrados = achados.slice(0, LIMITE_RESULTADOS);
      resultados.replaceChildren(
        el('p', {}, `${achados.length} carro(s) encontrado(s)` +
          (achados.length > mostrados.length ? ` — mostrando os ${mostrados.length} primeiros. Digite mais para filtrar.` : '. Toque no seu carro:')),
        el('div', { class: 'acoes' }, mostrados.map((m) => el('button', {
          type: 'button', class: 'btn secundario',
          onclick: () => {
            // Registro marcado na revisão do catálogo: mostra o motivo antes de preencher.
            if (m.revisar && m.revisar_motivos?.length) aviso(`Confira o motor deste carro: ${m.revisar_motivos.join(' ')}`, 'erro');
            api.preencher({ marca: m.marca, modelo: m.modelo, ano: m.ano ?? '', motor: m.motor ?? 'Motor não informado' });
          },
        }, el('span', {}, `${m.marca} ${m.modelo}${m.ano ? ` ${m.ano}` : ''}${m.motor ? ` · ${m.motor}` : ''}`),
          m.revisar ? el('span', { class: 'selo-revisar' }, 'conferir motor') : null))),
      );
    } else {
      resultados.replaceChildren(
        el('p', { class: 'vazio' }, 'Não encontramos esse carro no catálogo.'),
        botao({ texto: 'Cadastrar este carro mesmo assim', icone: 'mais', tipo: 'primario', onclick: () => api.preencher({ modelo: termo.trim(), motor: 'Motor não informado' }) }),
      );
    }
  }

  campoBusca.addEventListener('input', atualizar);
  topo.replaceChildren(el('section', { class: 'card busca' },
    el('label', { class: 'campo' }, el('span', { class: 'rotulo' }, icone('lupa', 22), ' Qual é o carro?'), campoBusca), resultados));
}

// Lê uma imagem escolhida pelo usuário (limite de tamanho e conversão para base64).
function lerImagem(arquivo) {
  return new Promise((ok, falha) => {
    if (arquivo.size > TAMANHO_MAXIMO) { falha({ texto: 'A foto passa de 5 MB. Escolha uma foto menor.' }); return; }
    const leitor = new FileReader();
    leitor.onload = () => ok({ nome: arquivo.name, mime: arquivo.type, dados: String(leitor.result).split(',')[1] });
    leitor.onerror = () => falha({ texto: 'Não foi possível ler a foto.' });
    leitor.readAsDataURL(arquivo);
  });
}

// Botão de foto: câmera do celular ou arquivo do computador.
function botaoEscolherFoto(aoEscolher) {
  const entrada = el('input', {
    type: 'file', accept: 'image/jpeg,image/png,image/webp', capture: 'environment', class: 'escondido', 'aria-label': 'Escolher foto',
    onchange: async (ev) => {
      const arquivo = ev.target.files?.[0];
      ev.target.value = '';
      if (!arquivo) return;
      try { aoEscolher(await lerImagem(arquivo)); } catch (e) { aviso(e.texto ?? 'Foto inválida.', 'erro'); }
    },
  });
  return el('span', {}, botao({ texto: 'Tirar ou escolher foto', icone: 'camera', tipo: 'primario', onclick: () => entrada.click() }), entrada);
}

// Fotos do cadastro novo: ficam guardadas até o carro ser salvo, e então são enviadas.
function fotosNoCadastro() {
  const pendentes = [];
  const grade = el('div', { class: 'fotos' });
  const status = el('p', { class: 'ajuda' });
  function desenhar() {
    grade.replaceChildren(...pendentes.map((f, i) => el('figure', { class: 'foto' },
      el('img', { src: `data:${f.mime};base64,${f.dados}`, alt: `Foto ${i + 1} do carro` }),
      el('figcaption', {}, botao({ texto: 'Tirar esta', icone: 'lixeira', tipo: 'link perigo', onclick: () => { pendentes.splice(i, 1); desenhar(); } })))));
    status.textContent = pendentes.length
      ? `${pendentes.length} foto(s) escolhida(s). Elas serão salvas junto com o carro.`
      : 'Opcional: tire ou escolha fotos do carro agora.';
  }
  desenhar();
  const node = el('section', { class: 'bloco-fotos' },
    el('h4', {}, icone('camera', 24), 'Fotos do carro'), status, grade,
    el('div', { class: 'acoes' }, botaoEscolherFoto((f) => { pendentes.push(f); desenhar(); })));
  return {
    node,
    async enviar(veiculo) {
      for (const f of pendentes.splice(0)) {
        await chamar('veiculos_foto_adicionar', { dados: { veiculo_id: veiculo.id, nome: f.nome, mime: f.mime, dados: f.dados } });
      }
    },
  };
}

// Fotos do carro: aparecem ao alterar um veículo. Podem ser tiradas com a câmera do celular ou escolhidas do computador.
function painelFotos(veiculo) {
  const grade = el('div', { class: 'fotos' });
  const status = el('p', { class: 'ajuda' });

  async function carregar() {
    try {
      const fotos = await chamar('veiculos_foto_listar', { veiculo_id: veiculo.id });
      const itens = await Promise.all(fotos.map(async (f) => {
        const d = await chamar('veiculos_foto_dados', { id: f.id });
        return el('figure', { class: 'foto' },
          el('img', { src: `data:${d.mime};base64,${d.dados}`, alt: `Foto do carro: ${f.nome}` }),
          el('figcaption', {}, botao({ texto: 'Remover', icone: 'lixeira', tipo: 'link perigo', onclick: async () => {
            if (!confirmar('Remover esta foto?')) return;
            await chamar('veiculos_foto_remover', { id: f.id }); carregar();
          } })));
      }));
      grade.replaceChildren(...itens);
      status.textContent = fotos.length ? `${fotos.length} foto(s) deste carro.` : 'Este carro ainda não tem fotos.';
    } catch (e) { aviso(e.texto, 'erro'); }
  }

  const entrada = el('input', {
    type: 'file', accept: 'image/jpeg,image/png,image/webp', capture: 'environment', class: 'escondido', 'aria-label': 'Escolher foto',
    onchange: async (ev) => {
      const arquivo = ev.target.files?.[0];
      ev.target.value = '';
      if (!arquivo) return;
      if (arquivo.size > TAMANHO_MAXIMO) { aviso('A foto passa de 5 MB. Escolha uma foto menor.', 'erro'); return; }
      const dados = await new Promise((ok, falha) => {
        const leitor = new FileReader();
        leitor.onload = () => ok(String(leitor.result).split(',')[1]);
        leitor.onerror = falha;
        leitor.readAsDataURL(arquivo);
      });
      try {
        await chamar('veiculos_foto_adicionar', { dados: { veiculo_id: veiculo.id, nome: arquivo.name, mime: arquivo.type, dados } });
        aviso('Foto adicionada.');
        carregar();
      } catch (e) { aviso(e.texto, 'erro'); }
    },
  });

  carregar();
  return el('section', { class: 'bloco-fotos' },
    el('h4', {}, icone('camera', 24), 'Fotos do carro'),
    status,
    grade,
    el('div', { class: 'acoes' },
      botao({ texto: 'Tirar ou escolher foto', icone: 'camera', tipo: 'primario', onclick: () => entrada.click() }),
      entrada));
}

// ---------- Histórico do veículo ----------
const SITUACAO = {
  CRIADA: 'Orçamento (aguardando início)',
  EM_ANDAMENTO: 'Em andamento',
  CONCLUIDA: 'Concluída e entregue',
  CANCELADA: 'Cancelada',
};
const dataBr = (iso) => (iso ? new Date(iso).toLocaleDateString('pt-BR') : '—');
const dataSimples = (aaaaMmDd) => (aaaaMmDd ? aaaaMmDd.split('-').reverse().join('/') : '—');
const kmTexto = (km) => (km === null || km === undefined ? 'não informado' : `${Number(km).toLocaleString('pt-BR')} km`);

// Uma ordem ou orçamento do carro: datas, peças e serviços, garantia e dados de quilometragem (editáveis).
function cartaoEntrada(e) {
  const o = e.ordem;
  const pecas = o.itens.filter((i) => i.tipo === 'peca');
  const servicos = o.itens.filter((i) => i.tipo === 'servico');
  const campoKm = (nome, rotulo, valor) => el('label', { class: 'campo' }, el('span', { class: 'rotulo' }, rotulo),
    el('input', { type: 'number', min: 0, max: 9999999, name: nome, value: valor ?? '' }));
  const form = el('form', { class: 'grade-campos' },
    campoKm('km_entrada', 'Quilometragem na entrada', e.km_entrada),
    campoKm('km_entrega', 'Quilometragem na entrega', e.km_entrega),
    el('label', { class: 'campo' }, el('span', { class: 'rotulo' }, 'Prazo previsto (data)'),
      el('input', { type: 'date', name: 'prazo_previsto', value: e.prazo_previsto ?? '' })),
    el('label', { class: 'campo' }, el('span', { class: 'rotulo' }, 'Garantia (dias após a entrega)'),
      el('input', { type: 'number', min: 0, max: 3650, name: 'garantia_dias', value: e.garantia_dias ?? '' })),
    el('label', { class: 'campo marcar' }, el('input', { type: 'checkbox', name: 'revisao', checked: e.revisao ? '' : null }),
      el('span', {}, 'Esta foi uma revisão programada')),
    el('label', { class: 'campo' }, el('span', { class: 'rotulo' }, 'Observações técnicas'),
      el('textarea', { name: 'observacoes_tecnicas', rows: 2 }, e.observacoes_tecnicas ?? '')),
    el('div', { class: 'acoes' }, botao({ texto: 'Salvar dados deste atendimento', icone: 'ok', tipo: 'primario', type: 'submit' })));
  form.addEventListener('submit', async (ev) => {
    ev.preventDefault();
    const f = new FormData(form);
    const num = (k) => (f.get(k) === '' ? null : Number(f.get(k)));
    try {
      await chamar('ordens_dados_veiculo_salvar', { id: o.id, dados: {
        km_entrada: num('km_entrada'), km_entrega: num('km_entrega'),
        prazo_previsto: f.get('prazo_previsto') || null, garantia_dias: num('garantia_dias'),
        revisao: f.get('revisao') === 'on', observacoes_tecnicas: f.get('observacoes_tecnicas') || null,
      } });
      aviso('Dados do atendimento salvos.');
    } catch (err) { aviso(err.texto, 'erro'); }
  });
  return el('li', { class: `linha-tempo-item situacao-${o.status.toLowerCase()}` },
    el('div', { class: 'linha-tempo-data' }, dataBr(o.data_entrada)),
    el('div', { class: 'linha-tempo-corpo' },
      el('h4', {}, `${o.tipo === 'OS' ? 'Ordem de serviço' : 'Venda'} nº ${o.id} — ${SITUACAO[o.status] ?? o.status}`,
        e.revisao ? el('span', { class: 'selo-revisar' }, 'Revisão') : null),
      el('p', {}, o.descricao ? `Descrição: ${o.descricao}` : 'Sem descrição.'),
      el('p', {}, `Entrada: ${dataBr(o.data_entrada)} · Prazo previsto: ${dataSimples(e.prazo_previsto)} · Entrega: ${dataBr(o.data_conclusao)}`),
      el('p', {}, `Garantia: ${e.garantia_dias ? `${e.garantia_dias} dias, até ${dataSimples(e.garantia_ate)}` : 'não informada'}`),
      el('p', {}, `Quilometragem: entrada ${kmTexto(e.km_entrada)} · entrega ${kmTexto(e.km_entrega)}`),
      el('p', {}, `Peças: ${pecas.length ? pecas.map((i) => `${i.item_nome} (${i.quantidade})`).join(', ') : 'nenhuma'}`),
      el('p', {}, `Serviços: ${servicos.length ? servicos.map((i) => i.item_nome).join(', ') : 'nenhum'}`),
      el('p', {}, `Valor: ${moeda(o.valor_total)}`),
      e.observacoes_tecnicas ? el('p', {}, `Observações técnicas: ${e.observacoes_tecnicas}`) : null,
      el('details', { class: 'detalhe-km' }, el('summary', {}, 'Registrar ou corrigir quilometragem e garantia'), form)));
}

function painelHistorico(veiculo) {
  const area = el('div', { class: 'linha-do-tempo' }, el('p', { class: 'ajuda' }, 'Carregando histórico...'));
  const seccao = el('section', { class: 'bloco-fotos', 'aria-label': 'Histórico do veículo' },
    el('h4', {}, icone('ordens', 24), 'Histórico do veículo'),
    el('p', { class: 'ajuda' }, 'Mostra as ordens e orçamentos deste carro, do mais recente para o mais antigo. Cada atendimento guarda a sua quilometragem; registrar uma nova não muda as antigas.'),
    area);
  (async () => {
    try {
      const h = await chamar('veiculos_historico', { veiculo_id: veiculo.id });
      area.replaceChildren(
        el('p', { class: 'texto-grande' }, h.km_mais_recente !== null ? `Maior quilometragem registrada: ${kmTexto(h.km_mais_recente)}` : 'Nenhuma quilometragem registrada ainda.'),
        h.entradas.length
          ? el('ol', { class: 'linha-do-tempo-lista' }, h.entradas.map(cartaoEntrada))
          : el('p', { class: 'vazio' }, 'Este carro ainda não tem ordens nem orçamentos.'));
    } catch (e) { area.replaceChildren(el('p', { class: 'erro-campo' }, e.texto ?? 'Não foi possível carregar o histórico.')); }
  })();
  return seccao;
}

export const render = crudPage({
  titulo: 'Veículos',
  icone: 'veiculos',
  singular: 'veículo',
  descricao: 'Carros dos clientes. Ao alterar um veículo você pode anexar fotos dele.',
  comando: 'veiculos',
  vazio: 'Nenhum veículo cadastrado ainda.',
  topo: buscaVeiculo,
  colunas: [
    { key: 'placa', label: 'Placa' },
    { key: 'modelo', label: 'Modelo' },
    { key: 'marca', label: 'Marca' },
    { key: 'ano', label: 'Ano' },
    { key: 'motor', label: 'Motor' },
    { key: 'cor', label: 'Cor' },
    { key: 'responsavel', label: 'Responsável' },
  ],
  campos: [
    { name: 'cliente_id', label: 'Dono do carro *', type: 'select', required: true, options: clientesOpcoes },
    { name: 'placa', label: 'Placa *', required: true, placeholder: 'ABC1D23' },
    { name: 'modelo', label: 'Modelo *', required: true },
    { name: 'marca', label: 'Marca' },
    { name: 'ano', label: 'Ano', type: 'number', min: 1900 },
    { name: 'motor', label: 'Motor' },
    { name: 'cor', label: 'Cor', placeholder: 'Ex.: Prata, Branco, Preto', list: 'cores-veiculo', sugestoes: CORES },
    { name: 'responsavel_id', label: 'Responsável pelo carro', type: 'select', options: responsaveisOpcoes },
  ],
  extraNovo: () => fotosNoCadastro(),
  extraEdicao: (veiculo) => el('div', {}, painelFotos(veiculo), painelHistorico(veiculo)),
});
