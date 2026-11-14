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

## O que já existe (versão 0.7 — reescrita em Rust)

A aplicação foi reescrita em Rust (interface Slint), com um **serviço do Windows** (conta de sistema) que faz as
alterações depois da confirmação. Sem o serviço instalado, a própria aplicação faz o mesmo papel: lê sem permissões
especiais e pede para reiniciar como administrador quando uma alteração o exige.

| Área | Estado |
|---|---|
| Resumo, diagnóstico Realtek/DisplayPort com evidências e ações propostas, relatórios (texto/JSON, dados sensíveis ocultados) | Feito |
| Dispositivos: reiniciar, ativar, desativar, desinstalar, escolher/reverter controlador, sempre com as regras de proteção | Feito |
| Controladores: atualizações do Windows Update, limpeza com impacto, Driver Store, cópias de segurança, bloqueio de instalação | Feito |
| Áudio: predefinido, volume, sem som, reiniciar serviços de áudio, perfil "DisplayPort preferido" | Feito |
| Ecrãs (ligações, modo, EDID) | Feito (só leitura) |
| Rede: partilha de Internet (NAT, DHCP, DNS, firewall, Hotspot Móvel) e **proteção contra fugas da VPN** por filtros WFP que bloqueiam se a VPN cair | Feito |
| Diário de alterações com reversão (recusa reverter o que foi alterado por outra via) | Feito |
| Atualização automática assinada (ECDSA P-256) a partir do GitHub; instalador MSI com o serviço | Feito |

**Honestidade sobre os testes:** a lógica tem testes automáticos (mais de 290) e as leituras do Windows, o serviço, o
relatório e as regras de firewall já correram num Windows real no GitHub. As ações que **alteram o sistema** estão
compiladas e testadas com dados simulados, mas **ainda não foram validadas em hardware real**; o registo está em
`docs/TESTING.md`.

## O que falta ou ficou limitado nesta versão

* Nos PCs sem o NAT do Windows (sem Hyper-V) a partilha usa o ICS do Windows: rede fixa 192.168.137.0/24, sem
  proteção contra fugas nem regras por dispositivo.
* Mudar o modo/topologia dos ecrãs, formatos de áudio, modo de teste de controladores, "resolver tudo" com uma só
  confirmação, ícone na bandeja (o perfil de áudio só atua com a aplicação aberta).
* Assinatura de código do executável (evita o aviso do SmartScreen) e validação em hardware real de cada ação.

## Regras que não mudam

Só mexe no que o bitBybit criou ou registou no diário; nunca desativa a Firewall do Windows, nunca bloqueia o Windows
Update, nunca limpa o Driver Store por inteiro, nunca remove controladores críticos, nunca contorna a assinatura de
controladores, nunca altera a rota predefinida sem autorização, nenhuma chave privada no repositório.

Documentação técnica: [`CLAUDE.md`](CLAUDE.md), [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md),
[`docs/IMPLEMENTATION_PLAN.md`](docs/IMPLEMENTATION_PLAN.md), [`docs/TESTING.md`](docs/TESTING.md),
[`docs/UPDATES.md`](docs/UPDATES.md), [`docs/RUST_MIGRATION_MATRIX.md`](docs/RUST_MIGRATION_MATRIX.md).
