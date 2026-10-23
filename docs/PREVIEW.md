# bitBybit 0.5 — pré-visualização (atualizações automáticas pelo GitHub)

Esta versão mantém todo o diagnóstico da 0.1 e acrescenta as primeiras **ações reversíveis** para o problema do
Realtek / DisplayPort. Nada é alterado sem a sua confirmação, e cada alteração fica registada no separador
**Alterações do bitBybit**, de onde a pode reverter.

## Como executar

1. Copie `bitBybit.exe` para uma pasta à sua escolha (por exemplo, `Transferências`).
2. Faça duplo clique. Como o executável **não está assinado digitalmente**, o Windows SmartScreen pode mostrar
   "O Windows protegeu o computador": clique em **Mais informações › Executar mesmo assim**.
   (Não é preciso instalar o .NET: o executável é autónomo.)
3. A recolha começa automaticamente (5–30 segundos).
4. Para ver também o registo de instalação de drivers (`setupapi.dev.log`), clique em **Reabrir como administrador**.

Requisitos: Windows 10 1809 ou posterior, ou Windows 11, de 64 bits.

## Novidades da 0.5 — atualizações automáticas

* O bitBybit procura versões novas no GitHub ao abrir (no máximo a cada 6 horas). Quando há uma, aparece um botão de
  transferência aceso na barra de cima: um clique descarrega, verifica a assinatura digital e reinicia já atualizado.
* **Definições › Atualizações**: canal **Testes** (cada alteração publicada) ou **Estável**, "Procurar agora" e notas da
  versão.
* Segurança: só versões do repositório do bitBybit, com a assinatura verificada com a chave incluída na aplicação, e
  nunca uma versão mais antiga. Se algo não bater certo, a atualização é recusada.

## Novidades da 0.4 — controlar, não só ler

* **Clique num dispositivo** (página Dispositivos) para abrir a janela do dispositivo:
  * **Reiniciar**, **Ativar**, **Desativar**, **Desinstalar** (o pacote do controlador fica no computador);
  * **Mostrar outros controladores**: a lista de controladores compatíveis que já estão no computador (como
    "Procurar no computador › Escolher numa lista" do Gestor de Dispositivos) com **Instalar este** — muda só esse
    dispositivo e faz cópia do pacote atual;
  * **Reverter controlador** (volta ao anterior, se o Windows o guardou).
  * Cada ação diz porque está ou não disponível e o risco. Dispositivos de que o Windows precisa para arrancar
    (sistema, discos, teclado/rato, USB, firmware) nunca são alterados; placas gráficas nunca são desativadas nem
    desinstaladas.
* **Procurar alterações de hardware** (botão na página Dispositivos).
* **Driver Store**: agora mostra também os pacotes de terceiros que **nenhum dispositivo usa**, com **Remover**
  (faz cópia primeiro; o Windows recusa se algum dispositivo o estiver a usar).
* **Áudio**: **Predefinir** e **Ativar** diretamente nos cartões.
* Tudo o que muda fica em **Alterações do bitBybit** e pode ser revertido: ativar/desativar, troca de controlador,
  reversão de controlador, remoção de pacote (volta a ser adicionado a partir da cópia).
* **Caso do som do monitor**: quando o som funciona com o controlador genérico do Windows, o bitBybit propõe remover
  o pacote antigo do fabricante para o Windows não voltar a trocá-lo. A conclusão sobre o "modo DVI" foi corrigida:
  o som pode funcionar mesmo quando o Windows indica DVI.
* O controlador de cada dispositivo passa a ser lido como o Gestor de Dispositivos o mostra (antes podia aparecer um
  pacote incluído, como wdmaudio.inf, em vez do controlador real).

## Novidades da 0.3

* **Novo aspeto**, inspirado no design do OrbisLink: temas **Escuro**, **Vidro**, **Claro** e **Personalizado**
  (botões no canto superior direito; a escolha fica guardada). Em **Definições** pode escolher a cor de destaque e
  ajustar cada cor do tema personalizado.
* **Barra superior** com o nome do PC, a versão do Windows, um resumo do diagnóstico e quatro indicadores
  (Administrador, Windows Update, Restauro do Sistema, Internet — passe o rato para ver o detalhe).
* **Páginas**: Caso Realtek / DisplayPort, Dispositivos (cartões, pesquisa, filtros por categoria, problemas
  primeiro), Atualizações (procura de drivers no Windows Update, **só leitura**), Driver Store, Cópias de segurança,
  Áudio, Ecrãs, Rede, Histórico, Alterações, Sistema e Registo (log).
* **Listas em cartões agrupados** (0.3.2): Driver Store por categoria, Áudio por estado (a funcionar, sem nada
  ligado, desativados, não presentes), Ecrãs, Rede por estado, Histórico, Alterações e Sistema.
* O resumo de problemas na barra superior abre a lista de problemas e avisos com as opções para os resolver (0.3.1).
* Quando o monitor está em modo DVI e as saídas de som HDMI/DisplayPort desaparecem, o bitBybit mostra um único aviso
  com a causa (a ligação DVI) em vez de vários "problemas" (0.3.2).
* As ações e o diagnóstico da 0.2 continuam iguais.
* Cada versão enviada para o GitHub gera automaticamente um executável de teste (separador *Actions › Artifacts*,
  nome `bitBybit-<versão>-run<N>-<commit>-win-x64-unsigned`) e capturas de ecrã da interface.

## Novidades da 0.2

* **Botão "Executar…"** nas ações propostas do separador *Caso Realtek / DisplayPort*. Cada ação mostra o impacto e
  o risco e pede confirmação:
  * **Tornar o DisplayPort o dispositivo predefinido** (risco baixo, sem administrador). Fica registado o
    dispositivo anterior para poder reverter.
  * **Ativar o ponto de saída DisplayPort** se estiver desativado (usa uma interface do Windows não documentada).
  * **Impedir que o Windows volte a instalar o driver Realtek** (precisa de administrador / UAC):
    1. faz cópia de segurança do pacote do driver Realtek (`%ProgramData%\bitBybit\driver-backups`);
    2. cria uma política do Windows que bloqueia a instalação de drivers **só para o ID de hardware exato do codec
       Realtek** (nunca para o áudio da placa gráfica, nunca para IDs genéricos);
    3. opcionalmente desinstala já o dispositivo Realtek (o pacote fica no Driver Store).
  * **Reiniciar o dispositivo de áudio** (só dispositivos de áudio presentes; administrador).
* **Perfil "Áudio DisplayPort preferido"**: com o som DisplayPort a funcionar, clique em **Criar perfil**. O bitBybit
  fica em segundo plano (ícone junto ao relógio, arranca com a sua sessão do Windows) e, quando o dispositivo
  DisplayPort reaparece (após reiniciar, ligar o monitor ou acordar o PC), volta a torná-lo predefinido.
  Respeita as suas escolhas manuais, tem limite de tentativas e nunca mexe em drivers. **Remover perfil** desliga tudo.
* **Separador "Alterações do bitBybit"**: lista tudo o que o bitBybit alterou, com **Reverter**. A reversão só é feita
  se o objeto continuar como o bitBybit o deixou; se outra pessoa/programa o alterou entretanto, aparece
  "Alterado externamente (não revertido)" e nada é mexido.

## Teste sugerido

1. Abra o bitBybit, veja o separador *Caso Realtek / DisplayPort* e clique em **Exportar relatório**.
2. Se o DisplayPort estiver ativo, clique em **Criar perfil**.
3. Se aparecer "O Windows ainda enumera o codec Realtek", use **Executar…** nessa conclusão e escolha se quer
   desinstalar já o dispositivo. Reinicie quando o Windows pedir.
4. Depois de reiniciar, abra o bitBybit: a comparação com a recolha anterior mostra se o Realtek voltou e se o
   DisplayPort continua presente e predefinido. Exporte o relatório.
5. Repita o reinício 2–3 vezes. Envie-me os relatórios `.txt`/`.json` se algo não correr como esperado.
6. Para desfazer: separador **Alterações do bitBybit › Reverter** (o bloqueio do driver é removido e o Windows
   volta a procurar o dispositivo) e **Remover perfil**.

## Onde ficam os dados

* `%LocalAppData%\bitBybit`: perfil de áudio, registo de alterações do utilizador, recolhas anteriores, registos (logs).
* `%ProgramData%\bitBybit`: registo de alterações do computador (políticas/dispositivos), cópias de segurança de
  drivers e resultados das operações de administrador (pasta protegida: só administradores escrevem).
* `Documentos\bitBybit`: relatórios exportados. Os relatórios ocultam números de série e parte dos endereços MAC.

## Linha de comandos (opcional)

`bitbybit-cli.exe diagnose --out C:\caminho\relatorio` gera os mesmos relatórios sem interface (só leitura).

## Limitações conhecidas desta versão

* As ações da 0.4 (ativar/desativar, trocar/reverter controlador, remover pacote) foram compiladas para Windows e
  testadas com simulações; no runner Windows do GitHub só a **listagem** de controladores compatíveis é executada (só
  leitura). **Ainda não foram executadas num PC real** — teste primeiro em dispositivos que não sejam críticos.
* As ações da 0.2 foram testadas com simulações (testes unitários) e compiladas para Windows; o diagnóstico correu num
  runner Windows Server do GitHub Actions (sem áudio nem monitores). **Ainda não foram validadas em hardware real** —
  o seu PC é o primeiro teste real.
* A política de bloqueio de instalação de drivers é respeitada pelo Windows 10/11 Pro/Enterprise/Education; no
  **Windows Home** o Windows pode ignorá-la. Depois de reiniciar, o bitBybit indica se o Realtek voltou.
* O módulo de rede (hotspot, partilha, VPN, firewall) ainda não está incluído; a classificação de interfaces de rede
  é uma estimativa.
* Não assinado digitalmente (Authenticode); ainda sem instalador nem atualizações automáticas.
* O novo aspeto foi verificado em capturas de ecrã geradas no runner Windows do GitHub; ainda não num ecrã real.
  A fila de operações, o assistente de primeira utilização e a instalação/reversão de drivers pela interface ficam
  para as próximas versões.
