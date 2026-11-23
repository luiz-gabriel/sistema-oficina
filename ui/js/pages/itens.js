import { crudPage } from '../crud.js';
import { moeda } from '../components.js';

export const render = crudPage({
  titulo: 'Peças e Serviços',
  paginada: true,
  icone: 'pecas',
  singular: 'peça ou serviço',
  descricao: 'Peças vendidas (controlam estoque) e serviços cobrados (mão de obra).',
  comando: 'itens',
  vazio: 'Nenhuma peça ou serviço cadastrado ainda.',
  colunas: [
    { key: 'nome', label: 'Nome' },
    { key: 'tipo', label: 'Tipo', render: (i) => (i.tipo === 'peca' ? 'Peça' : 'Serviço') },
    { key: 'preco_venda', label: 'Valor', render: (i) => moeda(i.preco_venda) },
    { key: 'quantidade', label: 'Em estoque', render: (i) => (i.tipo === 'peca' ? i.quantidade : '—') },
  ],
  campos: [
    { name: 'nome', label: 'Nome *', required: true },
    { name: 'tipo', label: 'É uma *', type: 'select', required: true, options: [['peca', 'Peça'], ['servico', 'Serviço']] },
    { name: 'codigo', label: 'Código de barras ou referência' },
    { name: 'preco_custo', label: 'Quanto custou', type: 'number', step: '0.01', min: 0 },
    { name: 'preco_venda', label: 'Valor cobrado *', type: 'number', step: '0.01', min: 0, required: true },
    { name: 'quantidade', label: 'Quantas tem no estoque', type: 'number', min: 0 },
    { name: 'estoque_minimo', label: 'Avisar quando tiver menos que', type: 'number', min: 0 },
  ],
});
