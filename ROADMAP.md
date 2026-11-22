# Roadmap

## Objetivo

**Substituir o `explorer.exe` como shell do Windows 11**, com o visual do Win11 reproduzido à risca e o mínimo de RAM e CPU. O `fluent-win` é a camada visual e de controles; o shell vem em cima dela.

Marco inicial: logar, ver barra de tarefas, Iniciar, bandeja e desktop, abrir e alternar aplicativos e desligar o PC, **sem nenhum `explorer.exe` rodando**.

Legenda: ✅ feito e verificado · 🟡 parcial · ⬜ não começado · ❓ incógnita a pesquisar antes de implementar.

---

## Estado atual (2026-11-22)

228 testes passando, clippy limpo, ~48 MB de RAM e 0 ms de CPU parado no exemplo (o `explorer.exe` foi medido em 327,7 MB).

| Área | Estado |
|---|---|
| Janela: Mica/Acrylic, cantos, barra de título escura, backdrop transitório | ✅ |
| Renderização: Direct2D/DirectWrite sobre DirectComposition, texto Segoe UI Variable, sem alocação por frame, recuperação de GPU perdida | ✅ |
| Tema: tokens do WinUI, acento e tema do sistema, alto contraste, 125% e 150% (inclusive trocando ao vivo) | ✅ |
| Controles: Button (padrão, accent, subtle, alternância), ToggleSwitch, CheckBox (3 estados), Slider | ✅ |
| Flyout: posicionamento do WinUI, animação do sistema, popup acrílico, light dismiss | ✅ |
| `Panel`: foco, captura, Tab, eventos | ✅ |
| Layout: empilhamento horizontal/vertical com medida e `Spacing` do `StackPanel` | ✅ |
| UI Automation: Invoke, Toggle, RangeValue, eventos de propriedade; Narrador testado | ✅ (NVDA ❓ não testado; conteúdo do flyout 🟡 sem provider) |
| Shell (barra de tarefas, Iniciar, bandeja, desktop) | ⬜ |

---

## Fase 1 — Controles e infraestrutura que faltam

O shell precisa disto antes de existir. Ordem sugerida (cada item: TDD, README, commit, revisão):

- [x] **Layout automático** (`layout::Stack`: empilhamento horizontal/vertical, `Spacing` e medida dos filhos). As fileiras de botões do exemplo já não são posicionadas à mão.
- [x] **MenuFlyout** (`menu_flyout::MenuFlyout`): itens, separadores, atalho, habilitado/desabilitado, setas/Home/End/Espaço/Enter e clique com captura, no popup do `flyout`. Falta submenu, ícone, mnemonic, clique direito e UIA próprio.
- [x] **ToolTip** (`tooltip::ToolTip` + `tooltip::Host`): pincéis/tipografia do `DefaultToolTipStyle`, atraso e duração lidos de `SPI_GETMOUSEHOVERTIME`/`SPI_GETMESSAGEDURATION`, fade e posicionamento sobre o ponteiro. Falta offsets, conteúdo rico e UIA próprio; SafeZone (casco convexo) já incluído.
- [ ] **ListView / GridView** (lista de apps do Iniciar, itens fixados), com virtualização.
- [ ] **ScrollViewer** (rolagem por roda e barra).
- [ ] **TextBox** (busca do Iniciar): cursor, seleção, área de transferência, IME, UIA Text/Value. É o item mais caro.
- [ ] **RadioButton, ProgressRing, ProgressBar**.
- [ ] **Ícones reais**: extrair ícone de app e arquivo (`IShellItemImageFactory`) e desenhar como bitmap D2D, com cache.
- [ ] **UIA do conteúdo de flyouts e popups** (o Narrador hoje não lê o que está dentro).
- [ ] **Cache do layout de texto** (`IDWriteTextLayout`), hoje refeito a cada frame.
- [ ] **Teste com NVDA**.

---

## Fase 2 — Barra de tarefas rodando ao lado do Explorer

Modo de desenvolvimento **seguro**: o shell roda como um aplicativo comum, com o `explorer.exe` ainda vivo. Nada de registro nem de troca de shell ainda.

- [ ] Janela da barra como **appbar** (`SHAppBarMessage`: `ABM_NEW`, `ABM_QUERYPOS`, `ABM_SETPOS`) reservando a área de trabalho.
- [ ] **Lista de janelas**: `EnumWindows` + filtro de janelas "de tarefa" (visíveis, sem dono, sem `WS_EX_TOOLWINDOW`) e atualização por `RegisterShellHookWindow` (mensagem `SHELLHOOK`: `HSHELL_WINDOWCREATED`, `WINDOWDESTROYED`, `WINDOWACTIVATED`, `FLASH`, `REDRAW`, `MONITORCHANGED`, entre outros; confirmado no Learn).
- [ ] Botões de tarefa: ícone, estado ativo, piscar, ativar/minimizar/restaurar ao clicar, menu de contexto da janela.
- [ ] Relógio e data, com formato do sistema.
- [ ] Vários monitores e DPI por monitor.
- [ ] Hospedar a **área de notificação** (bandeja). ❓ Como o `Shell_NotifyIcon` chega ao shell, qual janela o recebe e como reenviar os ícones quando a barra é recriada (mensagem `TaskbarCreated`): o Learn só diz "envia uma mensagem para a área de status da barra de tarefas"; o resto precisa ser verificado.
- [ ] Barra de tarefas com o `explorer.exe` desligado numa **conta de teste ou VM** (ver "Segurança e recuperação").

---

## Fase 3 — Iniciar, Configurações rápidas, Central de notificações

- [ ] **Iniciar**: fixados (grade), recomendados, lista de todos os apps, busca, botão de energia. ❓ Fonte da lista de apps e dos fixados (pastas do Menu Iniciar, `AppsFolder`, apps empacotados).
- [ ] **Configurações rápidas**: Wi-Fi, Bluetooth, modo avião, brilho, volume (o Slider e o ToggleButton já existem). ❓ APIs de cada um (WinRT `Radios`, `IAudioEndpointVolume`, brilho por WMI/DDC).
- [ ] **Central de notificações e calendário**. ❓ Como receber as notificações dos apps (`UserNotificationListener` exige permissão e identidade de pacote).
- [ ] Atalhos globais: tecla Windows, Win+D, Win+E, Win+L, Alt+Tab. ❓ Quais o Winlogon/explorer tratam e quais o shell precisa registrar.

---

## Fase 4 — Ser o shell de verdade

- [ ] **Registrar como shell**: valor `Shell` em `HKCU\Software\Microsoft\Windows NT\CurrentVersion\Winlogon` (por usuário; o padrão em `HKLM\...\Winlogon\Shell` é `explorer.exe` e fica intocado). O processo inicia com `C:\Windows\System32` como diretório de trabalho. Fonte: respostas da comunidade, não documentação oficial: ❓ confirmar em uma VM.
- [ ] **Desktop**: papel de parede, ícones da área de trabalho, menu de contexto, arrastar e soltar. ❓ Qual janela hospeda o desktop (classe `Progman`/`WorkerW`) e como usar `IShellView` sem o `explorer.exe`.
- [ ] **Bandeja completa** (ícones, balões, menus, ocultar/mostrar), compatível com os apps existentes.
- [ ] **Inicialização de apps**: ❓ quem processa as chaves `Run` e a pasta Startup quando o shell não é o Explorer (a suspeita é que seja o próprio shell, então ele precisa fazer isso).
- [ ] **Sessão**: bloquear, trocar usuário, sair, reiniciar, desligar, suspender, hibernar.
- [ ] **Watchdog**: o `Shell Launcher` da Microsoft monitora a saída do shell e age conforme o código de saída (reinicia, por exemplo); o nosso precisa de um equivalente para nunca deixar o usuário sem shell.
- [ ] **Componentes do Win11 que dependem do Explorer** (host do menu Iniciar, host de experiências do shell, widgets, pesquisa). ❓ Quais sobrevivem sem o `explorer.exe` e quais precisam ser substituídos.

---

## Fase 5 — Paridade e polimento

- [ ] Miniaturas de janela e pré-visualização (DWM thumbnails), jump lists, agrupamento de janelas.
- [ ] Gerenciador de arquivos (janela de pasta). ❓ Se o `explorer.exe` ainda abre janelas de pasta quando outro shell está ativo (provável), e o que vale reimplementar.
- [ ] Pesquisa, widgets, visão de tarefas e áreas de trabalho virtuais.
- [ ] Configurações de aparência próprias (posição e tamanho da barra, alinhamento, tema).
- [ ] Empacotamento e instalador com desinstalação limpa (restaurar o shell).

---

## Segurança e recuperação

Trocar o shell pode deixar o login sem desktop. Regras do projeto:

1. **Nunca** alterar o `Shell` em `HKLM`; só por usuário (`HKCU`), em uma conta de teste ou numa VM, até a Fase 4 estar madura.
2. Antes de qualquer teste real: ponto de restauração, ou snapshot da VM.
3. Como voltar ao Explorer: `Ctrl+Alt+Del` → Gerenciador de Tarefas → Executar nova tarefa → `explorer.exe`; ou apagar o valor `Shell` do `HKCU` (por outra conta administradora, ou em modo de segurança).
4. O shell deve ter modo de recuperação embutido (atalho que abre o Gerenciador de Tarefas e o `explorer.exe`) e um watchdog.
5. Limite do `Shell Launcher` da Microsoft (Enterprise, Education e IoT): ele não suporta um shell que lança outro processo e termina. Nosso shell deve ser um processo único e de vida longa.

---

## Critério de pronto da v1

- [ ] Login → barra, Iniciar, bandeja e desktop aparecem sem `explorer.exe` em execução.
- [ ] Abrir, alternar, minimizar e fechar aplicativos; ícones de bandeja dos apps comuns funcionam.
- [ ] Desligar, reiniciar, bloquear e sair.
- [ ] Narrador lê barra, Iniciar e bandeja.
- [ ] RAM ociosa bem abaixo do `explorer.exe` (medido: 327,7 MB) e 0% de CPU parado.
- [ ] Sobrevive a troca de DPI, de tema, de monitor e a GPU perdida.

---

## Como contribuir com este roadmap

Cada item entra seguindo o fluxo do projeto: documentação oficial primeiro (marcar como "não verificado" o que vier de memória), testes antes do código, um commit por passo, README atualizado no mesmo commit, revisão antes de fechar. Itens ❓ viram pesquisa documentada no README antes de virarem código.
