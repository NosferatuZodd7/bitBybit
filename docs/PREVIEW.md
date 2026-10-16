# bitBybit 0.1 — pré-visualização de diagnóstico (só leitura)

Esta é a primeira versão executável do bitBybit, para testar no seu PC. **Não altera nada no sistema**: só lê
informação e analisa-a. As ações (mudar o dispositivo predefinido, bloquear a reinstalação do Realtek, reiniciar
dispositivos, partilha de rede, etc.) chegam em versões posteriores, sempre com confirmação explícita e reversíveis.

## Como executar

1. Copie `bitBybit.exe` para uma pasta à sua escolha (por exemplo, `Transferências`).
2. Faça duplo clique. Como o executável **não está assinado digitalmente**, o Windows SmartScreen pode mostrar
   "O Windows protegeu o computador": clique em **Mais informações › Executar mesmo assim**.
   (Não é preciso instalar o .NET: o executável é autónomo.)
3. A recolha começa automaticamente (5–30 segundos).
4. Para ver também o registo de instalação de drivers (`setupapi.dev.log`), clique em **Reabrir como administrador**.

Requisitos: Windows 10 1809 ou posterior, ou Windows 11, de 64 bits.

## O que a versão 0.1 faz

* **Caso Realtek / DisplayPort**: verifica se o Windows ainda enumera o codec Realtek (apesar da BIOS), se existem
  componentes/extensões Realtek, o histórico do Windows Update e do `setupapi.dev.log` para o Realtek, se a função
  de áudio HD da placa gráfica (NVIDIA/AMD/Intel) está presente, se o monitor está ligado por DisplayPort e se o EDID
  anuncia áudio, o estado do ponto de saída HDMI/DisplayPort (ativo / desligado / desativado / não presente) e se o
  dispositivo predefinido é o Realtek em vez do DisplayPort. Cada conclusão separa **factos** de **hipóteses**.
* **Comparação automática** com a recolha anterior (guardada em `%LocalAppData%\bitBybit\snapshots`): indica se houve
  reinício entre recolhas e o que mudou (ponto de saída DisplayPort desaparecido, dispositivo predefinido alterado,
  driver alterado, dispositivo da placa gráfica desaparecido).
* Listas de áudio (todos os estados), monitores e placas gráficas, dispositivos e drivers (incluindo não presentes,
  com filtros), rede (física/virtual/VPN — estimativa nesta versão), histórico de drivers e serviços.
* **Exportar relatório**: cria `Documentos\bitBybit\relatorio-bitbybit-<data>.txt` e `.json`. Os relatórios ocultam
  números de série de dispositivos e a parte específica dos endereços MAC.

## Teste sugerido para o problema do DisplayPort

1. Com o monitor DisplayPort ligado e o som a funcionar, abra o bitBybit e clique em **Exportar relatório**.
2. Reinicie o computador.
3. Volte a abrir o bitBybit (de preferência como administrador) e exporte de novo.
4. A conclusão "alteração(ões) relevante(s) desde a recolha anterior" mostra exatamente o que mudou.
5. Se quiser, envie-me os dois ficheiros `.txt` (ou `.json`) para eu analisar a causa.

## Linha de comandos (opcional)

`bitbybit-cli.exe diagnose --out C:\caminho\relatorio` gera os mesmos relatórios sem interface.

## Limitações conhecidas desta versão

* Só leitura; nenhuma reparação é executada.
* A classificação de interfaces de rede/VPN é uma estimativa (o módulo de rede completo está em desenvolvimento).
* Testado apenas em compilação e num runner Windows Server do GitHub Actions (sem áudio, monitores ou Wi-Fi);
  ainda **não foi validado em hardware real** com Windows 10/11.
* Não assinado digitalmente (Authenticode).
