# bitBybit

Aplicação pessoal e não comercial para **Windows 10/11 (64 bits)** que reúne num só sítio o diagnóstico e o controlo de
**hardware e controladores, áudio, vídeo/ecrãs e rede** (hotspot, partilha de ligação, VPN, firewall). A interface é em
português europeu (pt-PT); o código, os registos e a documentação técnica são em inglês.

## A ideia base

O Windows espalha estas tarefas por muitos sítios (Gestor de Dispositivos, Definições, painel do som, `pnputil`, Política
de Grupo, PowerShell…) e quase nunca explica **porquê** algo falha nem permite **voltar atrás** com segurança. O bitBybit
junta tudo numa janela com três regras:

1. **Primeiro provar, depois agir.** Cada conclusão mostra a evidência e distingue *facto*, *observação*, *inferência* e
   *hipótese*. Nunca apresenta uma suspeita como certeza.
2. **Nada muda sem confirmação.** O programa só *propõe* ações, com o impacto e o nível de risco. O utilizador confirma e o
   Windows pede autorização (UAC) para cada alteração ao sistema.
3. **Tudo é reversível e fica registado.** Antes de mexer, guarda o estado anterior (e copia o controlador); cada alteração
   vai para o diário e pode ser revertida, e o programa recusa reverter o que entretanto foi alterado por outra via.

Nasceu de um caso concreto: **o áudio pelo cabo DisplayPort/HDMI não aparecia** num PC com placa Realtek, porque o
Windows Update voltava a instalar o controlador Realtek. Esse caso (diagnóstico, bloqueio do controlador certo,
experimentar o controlador genérico) continua a ser o primeiro teste da aplicação; o resto cresceu à volta dele.

## Para que serve

* **Perceber o que se passa**: relatório do PC (dispositivos, controladores, áudio, ecrãs, rede, histórico de instalações
  de controladores) com as conclusões por ordem de gravidade e o que fazer em cada uma.
* **Controlar dispositivos e controladores**: reiniciar, ativar, desativar, desinstalar, escolher outro controlador já
  existente no PC, reverter para o anterior, **testar** um controlador (volta ao anterior ao fechar a aplicação).
* **Limpar o sistema com critério**: pacotes de controladores antigos ou sem hardware, dispositivos que já não existem —
  cada item com o impacto explicado, seleção múltipla e cópia de segurança antes de remover. Nunca toca em arranque,
  armazenamento, teclado, rato, USB ou chipset.
* **Atualizar controladores** pelo Windows Update (a única fonte), todos ou só os escolhidos, com ponto de restauro.
* **Resolver automaticamente** as conclusões seguras (risco baixo/médio, reversíveis, sem reiniciar) com uma só confirmação.
* **Atualizar-se a si própria** a partir das *releases* do repositório no GitHub (canal Testes ou Estável), com assinatura
  digital verificada; sem telemetria e sem servidores de terceiros.

## O que já existe (versão 0.6)

| Área | Estado |
|---|---|
| Diagnóstico Realtek/DisplayPort, evidências, relatórios (texto/JSON, dados sensíveis ocultados) | Feito |
| Dispositivos: ações protegidas por regras (nunca desativa/desinstala a placa gráfica nem dispositivos críticos) | Feito |
| Arquivo de controladores, cópias de segurança, limpeza com impacto, atualizações do Windows Update | Feito |
| Modo de teste de controladores, reversão automática, **Resolver automaticamente** | Feito |
| Diário de alterações com reversão (botão em destaque só quando há alterações ativas) | Feito |
| Interface pt-PT com 4 aspetos (escuro, vidro, claro, personalizado), logótipo e ícone | Feito |
| Atualização automática assinada (ECDSA P-256) a partir do GitHub | Feito |
| Perfil de áudio "DisplayPort preferido" em segundo plano (ícone na bandeja) | Feito |

**Honestidade sobre os testes:** a lógica de decisão tem testes automáticos (256) e a compilação, os testes e uma
verificação das janelas correm num Windows real no GitHub. As ações que **alteram o sistema** (instalar/reverter
controladores, limpeza, atualizações, modo de teste) estão compiladas e testadas com dados simulados, mas **ainda não
foram validadas em hardware real**; o registo do que foi validado onde está em `docs/TESTING.md`.

## O que falta implementar

**Módulo de rede (o grande bloco em falta)** — hoje só existem os modelos e a deteção, sem ecrãs nem ações:

* Hotspot Wi-Fi (Mobile Hotspot, Wi-Fi Direct, Hosted Network) e deteção honesta do que o PC suporta.
* Partilha de ligação à Internet (ICS, NAT do Windows, encaminhamento) com validador de topologia.
* Partilha de VPN **com proteção contra fugas** feita por filtragem de pacotes (WFP) que bloqueia tudo se a VPN cair —
  só se anuncia proteção quando isto estiver ativo.
* Servidor DHCP e DNS próprios (os motores existem; falta ligá-los e testá-los), firewall só com regras do bitBybit,
  lista de clientes ligados. Ponte/repetidor: apenas deteção e capacidade, o Windows não tem API documentada para criar.

**Infraestrutura**

* **Serviço do Windows** (conta de sistema) que substitua o ajudante elevado atual: o protocolo, a lista fechada de
  operações e a confirmação em duas fases já estão no código; falta o servidor do pipe com identificação do cliente.
* **Instalador MSI** (WiX) e assinatura de código do executável (evita o aviso do SmartScreen).
* Fila de operações visível, assistente do primeiro arranque, alterações de modo/topologia de ecrã e controlos de áudio
  mais finos (volume, formatos) no ecrã.
* Relatório de diagnóstico na linha de comandos mais completo e documentação final.

**Validação em hardware real**

* Repetir o caso Realtek/DisplayPort depois de reiniciar (persistência), e testar em PC real cada ação que altera o
  sistema, antes de as marcar como validadas.
* Ainda sem versão **Estável**: só existe o canal Testes; a primeira estável depende de um *tag* que só o dono publica.

## Regras que não mudam

Só mexe no que o bitBybit criou ou registou no diário; nunca desativa a Firewall do Windows, nunca bloqueia o Windows
Update, nunca limpa o Driver Store por inteiro, nunca remove controladores críticos, nunca contorna a assinatura de
controladores, nunca altera a rota predefinida sem autorização, nenhuma chave privada no repositório.

Documentação técnica: [`CLAUDE.md`](CLAUDE.md), [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md),
[`docs/IMPLEMENTATION_PLAN.md`](docs/IMPLEMENTATION_PLAN.md), [`docs/TESTING.md`](docs/TESTING.md),
[`docs/UPDATES.md`](docs/UPDATES.md), [`docs/PREVIEW.md`](docs/PREVIEW.md).
