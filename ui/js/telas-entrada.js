// Telas de entrada, na ordem em que aparecem: ativação, modo de funcionamento, administrador e login.
import { invocar } from './api.js';
import { el, aviso, botao, icone } from './components.js';

const cartao = (...filhos) => el('section', { class: 'entrada card-grande' }, ...filhos);

export function telaAtivacao(aoConcluir) {
  const input = el('input', {
    id: 'codigo', name: 'codigo', placeholder: 'XXXX-XXXX-XXXX-XXXX', autocomplete: 'off',
    maxlength: '19', required: '', autofocus: '',
    oninput: (e) => { e.target.value = e.target.value.toUpperCase(); },
  });
  const form = el('form', {
    onsubmit: async (ev) => {
      ev.preventDefault();
      try {
        const resultado = await invocar('ativar_sistema', { codigo: input.value.trim() });
        if (resultado?.fonte === 'relogio_local') {
          aviso('Sistema ativado. Sem internet, a data de ativação veio do relógio deste computador. Confira a data e a hora.', 'erro');
        } else {
          aviso('Sistema ativado com sucesso.');
        }
        aoConcluir();
      } catch (e) {
        aviso(e.texto, 'erro');
      }
    },
  },
    el('label', { class: 'campo' }, el('span', { class: 'rotulo' }, 'Código de ativação'), input,
      el('small', { class: 'ajuda' }, 'Digite os 16 caracteres do código que você recebeu.')),
    botao({ texto: 'Ativar o sistema', icone: 'chave', tipo: 'primario', type: 'submit' }),
  );
  return cartao(
    el('p', { class: 'passo' }, 'Passo 1 de 3'),
    el('h1', {}, icone('chave', 36), el('span', {}, 'Ativar o sistema')),
    el('p', { class: 'texto-grande' }, 'Este computador ainda não foi ativado. Digite o código de ativação para continuar.'),
    form,
  );
}

export function telaModo(estado, aoConcluir) {
  const opcoes = [
    { valor: 'servidor', titulo: 'Este computador é o principal', icone: 'config', texto: 'Guarda os dados da oficina. Os outros computadores se conectam a ele pela rede.' },
    { valor: 'cliente', titulo: 'Usar o computador principal', icone: 'setas', texto: 'Este computador usa os dados do computador principal da oficina.' },
    { valor: 'local', titulo: 'Só este computador', icone: 'inicio', texto: 'Sem rede. Os dados ficam somente aqui.' },
  ];
  let escolhido = 'servidor';
  const area = el('div', { class: 'opcoes' });
  const extra = el('div', { class: 'extra-modo' });
  const desenhar = () => {
    area.replaceChildren(...opcoes.map((o) => el('button', {
      type: 'button', class: `opcao ${escolhido === o.valor ? 'escolhida' : ''}`, 'aria-pressed': String(escolhido === o.valor),
      onclick: () => { escolhido = o.valor; desenhar(); },
    }, icone(o.icone, 30), el('strong', {}, o.titulo), el('span', {}, o.texto))));
    extra.replaceChildren(escolhido === 'cliente' ? blocoCliente() : '');
  };
  function blocoCliente() {
    const endereco = el('input', { name: 'endereco', placeholder: 'Ex.: 192.168.0.10:7878', autocomplete: 'off' });
    const lista = el('div', { class: 'achados' });
    return el('div', { class: 'bloco-cliente' },
      el('label', { class: 'campo' }, el('span', { class: 'rotulo' }, 'Endereço do computador principal'), endereco,
        el('small', { class: 'ajuda' }, 'Não sabe o endereço? Clique em "Procurar na rede" com o computador principal ligado.')),
      botao({ texto: 'Procurar na rede', icone: 'lupa', onclick: async () => {
        lista.replaceChildren(el('p', { class: 'texto-grande' }, 'Procurando...'));
        try {
          const achados = await invocar('rede_buscar');
          lista.replaceChildren(...(achados.length
            ? achados.map((a) => botao({ texto: `${a.computador} (${a.endereco}) — ${a.papel === 'servidor' ? 'principal' : 'cliente'}`, onclick: () => { endereco.value = a.endereco; } }))
            : [el('p', { class: 'texto-grande' }, 'Nenhum computador encontrado. Confira se o principal está ligado e na mesma rede.')]));
        } catch (e) { lista.replaceChildren(); aviso(e.texto, 'erro'); }
      } }),
      lista,
      el('div', { class: 'acoes' }, botao({ texto: 'Conectar e copiar os dados', icone: 'disco', tipo: 'primario', onclick: async () => {
        try {
          await invocar('config_modo', { modo: 'cliente', endereco: endereco.value.trim() });
          aviso('Dados copiados do computador principal.');
          aoConcluir();
        } catch (e) { aviso(e.texto, 'erro'); }
      } })),
    );
  }
  desenhar();
  return cartao(
    el('p', { class: 'passo' }, 'Passo 2 de 3'),
    el('h1', {}, 'Como este computador vai funcionar?'),
    el('p', { class: 'texto-grande' }, 'Escolha uma opção. Depois você pode mudar isso em Configurações.'),
    area, extra,
    el('div', { class: 'acoes' }, escolhido === 'cliente' ? null : botao({ texto: 'Continuar', icone: 'seta', tipo: 'primario', onclick: async () => {
      try {
        await invocar('config_modo', { modo: escolhido, endereco: null });
        aoConcluir();
      } catch (e) { aviso(e.texto, 'erro'); }
    } })),
  );
}

// Regras de senha mostradas antes de digitar: o usuário sabe o que é aceito.
const REGRAS = 'A senha precisa ter 8 caracteres ou mais, com letras e números, e não pode ser uma senha comum.';

export function telaAdministrador(aoConcluir) {
  const nome = el('input', { name: 'nome', autocomplete: 'name', required: '' });
  const login = el('input', { name: 'login', autocomplete: 'username', required: '' });
  const senha = el('input', { name: 'senha', type: 'password', autocomplete: 'new-password', required: '' });
  const confirma = el('input', { name: 'confirma', type: 'password', autocomplete: 'new-password', required: '' });
  const erro = el('p', { class: 'erro-campo', role: 'alert' });
  return cartao(
    el('p', { class: 'passo' }, 'Passo 3 de 3'),
    el('h1', {}, 'Criar o administrador'),
    el('p', { class: 'texto-grande' }, 'O administrador controla o sistema: cadastra funcionários e dá acesso a eles.'),
    el('form', {
      onsubmit: async (ev) => {
        ev.preventDefault();
        erro.textContent = '';
        if (senha.value !== confirma.value) { erro.textContent = 'As senhas não são iguais.'; return; }
        try {
          await invocar('criar_administrador', { nome: nome.value.trim(), login: login.value.trim(), senha: senha.value });
          aviso('Administrador criado. Agora entre no sistema.');
          aoConcluir();
        } catch (e) {
          erro.textContent = e.campos?.senha ?? e.campos?.login ?? e.texto;
        }
      },
    },
      el('label', { class: 'campo' }, el('span', { class: 'rotulo' }, 'Seu nome completo'), nome),
      el('label', { class: 'campo' }, el('span', { class: 'rotulo' }, 'Nome de usuário (para entrar)'), login, el('small', { class: 'ajuda' }, 'Use letras minúsculas, sem espaços. Ex.: joao')),
      el('label', { class: 'campo' }, el('span', { class: 'rotulo' }, 'Senha'), senha, el('small', { class: 'ajuda' }, REGRAS)),
      el('label', { class: 'campo' }, el('span', { class: 'rotulo' }, 'Digite a senha de novo'), confirma),
      erro,
      el('div', { class: 'acoes' }, botao({ texto: 'Criar administrador', icone: 'ok', tipo: 'primario', type: 'submit' })),
    ),
  );
}

// Botão para ver a senha que está sendo digitada (volta a ocultar ao clicar de novo).
function botaoVerSenha(senha) {
  const textoVer = el('span', {}, 'Ver senha');
  const verSenha = el('button', {
    type: 'button', class: 'btn link', 'aria-pressed': 'false', 'aria-label': 'Mostrar ou ocultar a senha',
    onclick: () => {
      const mostrar = senha.type === 'password';
      senha.type = mostrar ? 'text' : 'password';
      verSenha.setAttribute('aria-pressed', String(mostrar));
      textoVer.textContent = mostrar ? 'Ocultar senha' : 'Ver senha';
    },
  }, icone('lupa', 20), textoVer);
  return verSenha;
}

// Erro de entrada: bloqueio é avisado em destaque, para o usuário não tentar de novo sem saber.
function tratarErroEntrada(e, erro, senha) {
  erro.textContent = e.texto;
  if (/bloqueado/i.test(e.texto)) aviso(e.texto, 'erro');
  senha.value = '';
}

const avisoOffline = (estado) => (estado.modo === 'cliente' && estado.offline
  ? el('p', { class: 'aviso-inline' }, 'Este computador está em modo offline. Entre com a mesma senha de sempre.') : null);

// Entrada com a última sessão guardada (até 2 horas): pede só a senha desse usuário.
function formularioContinuar(estado, aoEntrar, ultima, outroUsuario) {
  const senha = el('input', { name: 'senha', type: 'password', autocomplete: 'current-password', required: '', autofocus: '' });
  const erro = el('p', { class: 'erro-campo', role: 'alert' });
  const m = ultima.ha_minutos;
  const quando = m < 1 ? 'há menos de 1 minuto' : `há ${m} minuto${m === 1 ? '' : 's'}`;
  return cartao(
    el('h1', {}, icone('cadeado', 36), el('span', {}, `Continuar como ${ultima.nome}`)),
    el('p', { class: 'texto-grande' }, `Último acesso ${quando}. Digite sua senha para continuar.`),
    el('p', { class: 'texto-grande' }, `Computador: ${estado.computador}`),
    avisoOffline(estado),
    el('form', {
      onsubmit: async (ev) => {
        ev.preventDefault();
        erro.textContent = '';
        try {
          await invocar('login', { login: ultima.usuario, senha: senha.value });
          aoEntrar();
        } catch (e) { tratarErroEntrada(e, erro, senha); }
      },
    },
      el('label', { class: 'campo' }, el('span', { class: 'rotulo' }, 'Senha'), senha),
      el('div', { class: 'acoes' }, botaoVerSenha(senha)),
      erro,
      el('div', { class: 'acoes' }, botao({ texto: 'Entrar', icone: 'seta', tipo: 'primario', type: 'submit' })),
    ),
    el('div', { class: 'acoes' }, botao({ texto: 'Entrar com outro usuário', icone: 'pessoa', onclick: outroUsuario })),
  );
}

// Formulário normal: usuário e senha. `voltar` existe quando veio de "Continuar como".
function formularioUsuario(estado, aoEntrar, voltar) {
  const login = el('input', { name: 'login', autocomplete: 'username', required: '', autofocus: '' });
  const senha = el('input', { name: 'senha', type: 'password', autocomplete: 'current-password', required: '' });
  const erro = el('p', { class: 'erro-campo', role: 'alert' });
  return cartao(
    el('h1', {}, icone('cadeado', 36), el('span', {}, voltar ? 'Entrar com outro usuário' : 'Entrar no sistema')),
    el('p', { class: 'texto-grande' }, `Computador: ${estado.computador}`),
    avisoOffline(estado),
    el('form', {
      onsubmit: async (ev) => {
        ev.preventDefault();
        erro.textContent = '';
        try {
          await invocar('login', { login: login.value.trim(), senha: senha.value });
          aoEntrar();
        } catch (e) { tratarErroEntrada(e, erro, senha); }
      },
    },
      el('label', { class: 'campo' }, el('span', { class: 'rotulo' }, 'Usuário'), login),
      el('label', { class: 'campo' }, el('span', { class: 'rotulo' }, 'Senha'), senha),
      el('div', { class: 'acoes' }, botaoVerSenha(senha)),
      erro,
      el('div', { class: 'acoes' }, botao({ texto: 'Entrar', icone: 'seta', tipo: 'primario', type: 'submit' })),
    ),
    voltar ? el('div', { class: 'acoes' }, botao({ texto: 'Voltar', icone: 'seta', onclick: voltar })) : null,
  );
}

// Tela de login. Se a última sessão deste computador tiver até 2 horas, oferece "Continuar como <nome>".
export function telaLogin(estado, aoEntrar) {
  const area = el('div', { class: 'entrada-area' });
  const ultima = estado.ultima_sessao;
  const desenhar = (continuar) => {
    area.replaceChildren(continuar && ultima
      ? formularioContinuar(estado, aoEntrar, ultima, () => desenhar(false))
      : formularioUsuario(estado, aoEntrar, ultima ? () => desenhar(true) : null));
  };
  desenhar(Boolean(ultima));
  return area;
}

