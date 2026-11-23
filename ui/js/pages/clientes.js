import { crudPage } from '../crud.js';
import { botao } from '../components.js';

export const render = crudPage({
  titulo: 'Clientes',
  icone: 'clientes',
  singular: 'cliente',
  descricao: 'Pessoas que trazem veículos para a oficina.',
  comando: 'clientes',
  vazio: 'Nenhum cliente cadastrado ainda.',
  colunas: [{ key: 'nome', label: 'Nome' }, { key: 'telefone', label: 'Telefone' }, { key: 'email', label: 'E-mail' }],
  // Leva à aba Veículos já com este cliente como dono, com a busca do catálogo disponível.
  // A tela de Clientes e Veículos tem id 'clientes' no app.js (a aba é escolhida por opcoes.aba).
  acoesLinha: (cliente, ctx) => [
    botao({
      texto: 'Cadastrar veículo deste cliente', icone: 'mais', tipo: 'link',
      onclick: () => ctx.irPara?.('clientes', { aba: 'veiculos', valores: { cliente_id: String(cliente.id) } }),
    }),
    botao({
      texto: 'Pontos de fidelidade', icone: 'seta', tipo: 'link',
      onclick: () => ctx.irPara?.('fidelidade', { cliente_id: cliente.id }),
    }),
  ],
  campos: [
    { name: 'nome', label: 'Nome completo *', required: true },
    { name: 'telefone', label: 'Telefone / WhatsApp', type: 'tel' },
    { name: 'email', label: 'E-mail', type: 'email' },
    { name: 'cpf', label: 'CPF / CNPJ' },
    { name: 'endereco', label: 'Endereço' },
    { name: 'observacoes', label: 'Observações', type: 'textarea' },
  ],
});
