// Funcionários: cadastro e concessão ou retirada de acesso. Tela só do administrador (o menu já esconde para os demais).
import { chamar } from '../api.js';
import { el, aviso, campo, tabela, botao, icone, cabecalhoPagina, confirmar } from '../components.js';
import { coerceDados } from '../crud.js';

const CAMPOS = [
  { name: 'nome', label: 'Nome completo *', required: true },
  { name: 'cargo', label: 'Função (ex.: mecânico, recepção)' },
  { name: 'telefone', label: 'Telefone', type: 'tel' },
];

export async function render(container, { estado }) {
  const admin = estado().logado.perfil === 'administrador';
  // Por segurança a tela é só do administrador; mantido o cálculo caso a tela seja aberta por outro caminho.
  const podeMexerNoAcesso = (f) => admin || f.perfil !== 'administrador';
  let editando = null;
  let acessoDe = null;
  const formArea = el('section', { class: 'card' });
  const listaArea = el('section', { class: 'card' });
  container.replaceChildren(
    cabecalhoPagina('Funcionários', 'equipe',
      'Cadastre quem trabalha na oficina. Depois, dê acesso ao sistema para quem precisar entrar.'),
    formArea, listaArea,
  );

  function desenharForm() {
    const valores = editando ?? {};
    const form = el('form', {
      onsubmit: async (ev) => {
        ev.preventDefault();
        const dados = coerceDados(CAMPOS, Object.fromEntries(new FormData(ev.target).entries()));
        try {
          if (editando) await chamar('funcionarios_atualizar', { id: editando.id, dados });
          else await chamar('funcionarios_criar', { dados });
          aviso('Funcionário salvo.');
          editando = null;
          desenharForm();
          carregar();
        } catch (e) { aviso(e.texto, 'erro'); }
      },
    },
      CAMPOS.map((c) => campo(c, valores[c.name])),
      el('div', { class: 'acoes' },
        botao({ texto: editando ? 'Salvar alterações' : 'Cadastrar funcionário', icone: editando ? 'ok' : 'mais', tipo: 'primario', type: 'submit' }),
        editando ? botao({ texto: 'Cancelar', icone: 'x', onclick: () => { editando = null; desenharForm(); } }) : null));
    formArea.replaceChildren(el('h3', {}, icone(editando ? 'lapis' : 'mais', 22), editando ? 'Alterar funcionário' : 'Novo funcionário'), form);
  }

  function formAcesso(f) {
    const login = el('input', { name: 'login', value: f.login ?? '', autocomplete: 'off', required: '' });
    const senha = el('input', { name: 'senha', type: 'password', autocomplete: 'new-password', required: '' });
    const perfil = el('select', { name: 'perfil' },
      el('option', { value: 'funcionario' }, 'Funcionário (usa o sistema)'),
      admin ? el('option', { value: 'administrador' }, 'Administrador (controla o sistema)') : null);
    const erro = el('p', { class: 'erro-campo', role: 'alert' });
    return el('form', { class: 'form-acesso',
      onsubmit: async (ev) => {
        ev.preventDefault();
        try {
          await chamar('funcionarios_dar_acesso', { id: f.id, dados: { login: login.value.trim(), senha: senha.value, perfil: perfil.value } });
          aviso(`Acesso liberado para ${f.nome}.`);
          acessoDe = null;
          carregar();
        } catch (e) { erro.textContent = e.campos?.senha ?? e.campos?.login ?? e.texto; }
      } },
      el('h4', {}, icone('chave', 22), `Acesso ao sistema — ${f.nome}`),
      el('label', { class: 'campo' }, el('span', { class: 'rotulo' }, 'Usuário para entrar'), login, el('small', { class: 'ajuda' }, 'Letras minúsculas, sem espaços.')),
      el('label', { class: 'campo' }, el('span', { class: 'rotulo' }, 'Senha'), senha, el('small', { class: 'ajuda' }, 'Mínimo 8 caracteres, com letras e números.')),
      el('label', { class: 'campo' }, el('span', { class: 'rotulo' }, 'Tipo de acesso'), perfil),
      erro,
      el('div', { class: 'acoes' },
        botao({ texto: 'Liberar acesso', icone: 'ok', tipo: 'primario', type: 'submit' }),
        botao({ texto: 'Cancelar', icone: 'x', onclick: () => { acessoDe = null; carregar(); } })));
  }

  async function carregar() {
    try {
      const lista = await chamar('funcionarios_listar');
      const linhas = lista.map((f) => ({
        ...f,
        acesso_texto: f.tem_acesso ? `Sim (${f.login}${f.perfil === 'administrador' ? ', administrador' : ''})` : 'Não',
        __acoes: el('span', { class: 'botoes-linha' },
          podeMexerNoAcesso(f) ? botao({ texto: f.tem_acesso ? 'Alterar acesso' : 'Dar acesso', icone: 'chave', tipo: 'link', onclick: () => { acessoDe = f.id; carregar(); } }) : null,
          podeMexerNoAcesso(f) && f.tem_acesso ? botao({ texto: 'Tirar acesso', icone: 'x', tipo: 'link perigo', onclick: async () => {
            if (!confirmar(`Tirar o acesso de ${f.nome}? Ele continua cadastrado.`)) return;
            await chamar('funcionarios_revogar_acesso', { id: f.id }); carregar(); } }) : null,
          botao({ texto: 'Alterar', icone: 'lapis', tipo: 'link', onclick: () => { editando = f; desenharForm(); formArea.scrollIntoView({ behavior: 'smooth' }); } }),
          podeMexerNoAcesso(f) ? botao({ texto: 'Excluir', icone: 'lixeira', tipo: 'link perigo', onclick: async () => {
            if (!confirmar(`Excluir ${f.nome}?`)) return;
            await chamar('funcionarios_remover', { id: f.id }); carregar(); } }) : null,
        ),
      }));
      const alvo = lista.find((f) => f.id === acessoDe);
      listaArea.replaceChildren(
        alvo && podeMexerNoAcesso(alvo) ? formAcesso(alvo) : null,
        el('h3', {}, 'Equipe'),
        tabela([
          { key: 'nome', label: 'Nome' },
          { key: 'cargo', label: 'Função' },
          { key: 'acesso_texto', label: 'Entra no sistema?' },
        ], linhas, 'Nenhum funcionário cadastrado ainda.'),
      );
    } catch (e) { aviso(e.texto, 'erro'); }
  }

  desenharForm();
  carregar();
}
