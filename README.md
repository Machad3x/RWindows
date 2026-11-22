# fluent-win

Visual do Windows 11 (Fluent) para janelas Win32 puras em Rust. A meta é imitar o Win11 à risca, com o mínimo de RAM e CPU.

É a camada visual que falta para shells como o [WinRust](https://github.com/Lemxy/WinRust), que usa GDI+ e por isso não tem antialiasing bom, Mica nem tipografia Segoe UI Variable.

## Uso

```powershell
cargo run --release --example demo
```

```rust
let theme = Theme::system();
backdrop::apply(hwnd, DWMSBT_MAINWINDOW, DWMWCP_ROUND, theme.dark)?; // janela com WS_EX_NOREDIRECTIONBITMAP
let mut surface = Surface::new(hwnd)?; // mut: end() recria tudo se a GPU for perdida
let body = surface.format(&theme::BODY)?;

surface.begin();
surface.fill_round(rect(16.0, 16.0, 216.0, 56.0), theme::CONTROL_RADIUS, theme.control_fill);
surface.text("Olá", &body, rect(28.0, 26.0, 200.0, 46.0), theme.text_primary);
surface.end()?;
```

## Arquitetura

| Módulo | Função |
|---|---|
| `backdrop` | Mica/Acrylic, cantos arredondados e barra de título escura via DWM (`DwmSetWindowAttribute`) |
| `gfx` | Direct2D + DirectWrite sobre um swap chain de DirectComposition com alfa pré-multiplicado. Pixels transparentes mostram o backdrop do DWM. |
| `theme` | Tokens de cor do WinUI, type ramp, raios de canto e acento do sistema |
| `easing` | Curvas e durações de animação do Win11, mais um `Tween` |

Única dependência: o crate `windows`. Não usa framework, webview nem GPUI.

## Decisões

Cada decisão registra o motivo e a fonte. "Não verificado" quer dizer que vem de memória ou inferência, sem fonte oficial.

| Decisão | Motivo | Fonte |
|---|---|---|
| Swap chain de **composição** (DirectComposition), não de HWND | Swap chain de HWND não aceita alfa, então o Mica não apareceria | Amostra `canvas` do windows-rs (`D2D1_ALPHA_MODE_PREMULTIPLIED`) |
| `DwmExtendFrameIntoClientArea` com margens -1 | O backdrop passa a cobrir toda a área cliente | Documentação do DWM |
| Texto com AA **grayscale** | ClearType não mistura sobre superfície transparente, e o WinUI também usa grayscale | Comportamento do Direct2D |
| Device D3D11 `SINGLETHREADED` + `PREVENT_INTERNAL_THREADING_OPTIMIZATIONS` | Working set 51.7 → 45.5 MB e threads 23 → 13, medidos | Commit `f1c984e` |
| Fallback para WARP | Funciona em VM sem GPU e com driver quebrado | Revisão Opus |
| Retângulos alinhados ao pixel físico | Imita o layout rounding do WinUI e evita bordas borradas em 125%/150% | Revisão Opus |
| Tokens de cor com os nomes das chaves do WinUI | Rastreabilidade: cada valor tem nome oficial | [`Common_themeresources_any.xaml`](https://github.com/microsoft/microsoft-ui-xaml/blob/main/controls/dev/CommonStyles/Common_themeresources_any.xaml) |
| `stroke_elevation`: borda de 1 DIP com gradiente linear absoluto nos 3 DIPs da borda (paradas 0.33/1.0), `Edge::Top` no escuro e `Edge::Bottom` no claro e em todo accent | Reproduz `ControlElevationBorderBrush`/`AccentControlElevationBorderBrush` (`MappingMode=Absolute`, `EndPoint 0,3`, `ScaleY=-1` quando invertido) | WinUI `Common_themeresources_any.xaml` |
| Larguras de borda arredondadas a pixels inteiros, no mínimo 1 (`snap_len`), e fundo `InnerBorderEdge` recuado pela mesma largura | Em 125%/150% a borda de 1 DIP daria 1.25/1.5 px, com emenda translúcida entre fundo e borda. O XAML arredonda `BorderThickness` no layout | Revisão Opus |
| `stroke_round` com largura e raio externo, igual a um `Border` XAML; `measure` mede texto numa linha | O foco usa bordas de 2 e 1 DIP; o Button dimensiona pelo texto | `FocusRectManager.cpp` (WinUI) |
| Dependência direta `windows-numerics` 0.3.1 | `Vector2` dos pontos do gradiente; já vinha como dependência do `windows`, sem crate novo na árvore | `Cargo.lock` |
| Tokens de estado (disabled, on-accent, stroke secundário, foco) | Necessários para Button e outros controles. `AccentFillColorSecondary/Tertiary` = acento com `Opacity` 0.9/0.8 (`with_opacity`) | WinUI `Common_themeresources_any.xaml` (dicionários Default e Light) |
| Durações 83/167/250 ms, curva `0,0,0,1` | `ControlFaster/Fast/NormalAnimationDuration`, `ControlFastOutSlowInKeySpline` | WinUI `Common_themeresources_any.xaml` |
| Idle bloqueado em `GetMessageW` e frames só com animação ativa | 0% de CPU parado | Medido |
| Exemplos soltam o `APP` (objetos D3D/DXGI/DComp) antes de `main` retornar | Destruídos nos destrutores de `thread_local`, durante o encerramento do processo, eles transformavam o código de saída em `0x87A` (`FACILITY_DXGI`). Com o fix, a saída é 0 | Medido (`WM_CLOSE` → código de saída) |
| Fonte: família tipográfica `Segoe UI Variable` via `IDWriteFactory6`, com `DWRITE_AUTOMATIC_FONT_AXES_OPTICAL_SIZE` e peso pelo eixo `wght` | É o que o XAML faz: `XamlAutoFontFamily` e tamanho óptico automático. Os nomes WWS ("… Small/Text/Display") eram uma aproximação | [Typography in Windows](https://learn.microsoft.com/windows/apps/design/signature-experiences/typography), [DWRITE_AUTOMATIC_FONT_AXES](https://learn.microsoft.com/windows/win32/api/dwrite_3/ne-dwrite_3-dwrite_automatic_font_axes), [CreateTextFormat (Factory6)](https://learn.microsoft.com/windows/win32/api/dwrite_3/nf-dwrite_3-idwritefactory6-createtextformat) |
| Recuperação de device perdido em `Surface::end` | `DXGI_ERROR_DEVICE_REMOVED`/`RESET` e `D2DERR_RECREATE_TARGET` recriam todos os recursos de device e pedem repaint (`InvalidateRect`). Formatos de texto continuam válidos porque o DirectWrite não depende de device. Shell aberto o dia todo passa por atualização de driver e RDP | [Handle device removed (DXGI)](https://learn.microsoft.com/windows/uwp/gaming/handling-device-lost-scenarios). Antes de recriar, solta o `IDCompositionTarget` antigo, porque o DirectComposition aceita só um por HWND. Testado: classificador e recriação no mesmo HWND (janela oculta). Não testado: remoção real de GPU |
| Tema e acento via `UISettings.GetColorValue` (WinRT), não pelo registro | API documentada e a mesma fonte que o WinUI usa. `AccentPalette` no registro não tem documentação. Modo escuro = foreground claro (`IsColorLight`). Acento: `AccentLight2` no escuro, `AccentDark1` no claro (`AccentFillColorDefault`). O windows-core entra no MTA sozinho quando recebe `CO_E_NOTINITIALIZED` | [Apply Windows themes](https://learn.microsoft.com/windows/apps/desktop/modernize/ui/apply-windows-themes), [UIColorType](https://learn.microsoft.com/uwp/api/windows.ui.viewmanagement.uicolortype), [Theming: accent palette](https://learn.microsoft.com/windows/apps/develop/ui/theming#accent-color-palette), WinUI `Common_themeresources_any.xaml` |
| Constantes `DARK.accent` #60CDFF / `LIGHT.accent` #005FB8 | Só usadas se o WinRT falhar. São os valores de design do WinUI. O Learn não publica as variações do azul padrão do Win11 | Não verificado |
| **Sem** `LineHeight` forçado: a linha usa as métricas naturais da fonte | Os estilos do WinUI não definem `LineHeight` (só `LineStackingStrategy=MaxHeight`), então o Win11 real usa a altura natural. Os valores 16/20/28… são da especificação de design | [`TextBlock_themeresources.xaml`](https://github.com/microsoft/microsoft-ui-xaml/blob/main/controls/dev/CommonStyles/TextBlock_themeresources.xaml), [LineStackingStrategy](https://learn.microsoft.com/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.linestackingstrategy) |

## Button

Reproduz o `Button` do WinUI (`DefaultButtonStyle` e `AccentButtonStyle`). A entrada segue o `ButtonBase` com `ClickMode.Release`, o padrão. Cada regra tem teste em `src/button.rs`.

| Regra | Fonte |
|---|---|
| Prioridade visual: Disabled > Pressed > PointerOver > Rest | `ButtonBase` (`UpdateVisualState`) |
| Apertar dentro: captura o ponteiro, Pressed, foco tipo ponteiro (sem anel) | `ButtonBase_Partial.cpp` |
| Com o botão apertado, sair dos limites (tolerância 0.05 DIP) desliga Pressed e voltar religa. O clique só sai se ainda estiver Pressed ao soltar | `OnPointerMoved` / `IsValidPointerPosition` |
| Space/Enter com foco: Pressed no key down, clique no key up. Outra tecla no meio cancela. Ignorado com o ponteiro capturado | `ButtonBaseKeyProcess.h` |
| Perder o foco cancela o press e solta a captura. Desabilitar zera Pressed e PointerOver | `OnLostFocus`, `OnIsEnabledChanged` |
| Anel de foco só com foco de teclado | [Keyboard interactions: focus visuals](https://learn.microsoft.com/windows/apps/develop/input/keyboard-interactions#focus-visuals) |

Desenho (`Button::draw`):

| Elemento | Valor | Fonte |
|---|---|---|
| Fundo padrão Rest/Over/Pressed/Disabled | `ControlFillColor` Default/Secondary/Tertiary/Disabled, dentro da borda (`BackgroundSizing=InnerBorderEdge`) | `Button_themeresources.xaml` |
| Texto padrão | `TextFillColor` Primary/Primary/Secondary/Disabled | idem |
| Borda padrão | Elevação (Rest/Over); `ControlStrokeColorDefault` sólida (Pressed/Disabled) | idem |
| Fundo accent | Acento, acento×0.9, acento×0.8, `AccentFillColorDisabled`, sob a borda (`OuterBorderEdge`) | idem + `Common_themeresources_any.xaml` |
| Texto/borda accent | `TextOnAccentFillColor*`; elevação invertida (Rest/Over), sem borda (Pressed/Disabled) | idem |
| Transição | Só o fundo, 83 ms linear (`BrushTransition`) | idem |
| Tamanho | Texto + padding `11,5,11,6` + borda 1, arredondado (altura 32 no Body) | `ButtonPadding`, `ButtonBorderThemeThickness` |
| Anel de foco | Externo a 3 DIPs do botão, 2 DIPs, raio 7 (`FocusStrokeColorOuter`); interno 2 DIPs para dentro, 1 DIP, raio 5 (`FocusStrokeColorInner`) | `FocusRectManager.cpp` (`AdjustCornerRadius`) |

Exemplo: `cargo run --release --example buttons` (Tab/Shift+Tab, Espaço/Enter, mouse com captura). Medido: 47.9 MB de working set, 0 ms de CPU parado em 3 s.

### SubtleButton

`Button::subtle(label, rect)` é o `SubtleButtonStyle` (botões da barra de tarefas, Iniciar, barra de título): sem fundo nem borda em repouso.

| Estado | Fundo = borda | Texto | Fonte |
|---|---|---|---|
| Rest / Disabled | `SubtleFillColorTransparent` (`#00FFFFFF`), sem borda | Primary / Disabled | `Button_themeresources.xaml` (`SubtleButton*`), `Common_themeresources_any.xaml` |
| PointerOver | `SubtleFillColorSecondary` | Primary | idem |
| Pressed | `SubtleFillColorTertiary` | Secondary | idem |

A borda repete a cor do fundo; com `InnerBorderEdge` o anel de 1 DIP e o fundo recuado formam uma superfície só em regime estável. Nos 83 ms da transição o anel já está na cor final e só o fundo anima, como no WinUI (o `BrushTransition` é só do `Background`). Mesma transição de 83 ms, foco e padding do Button. No alto contraste os valores são os do Button padrão (dicionário `HighContrast`, mesmas chaves resolvidas).

Exemplo: "Iniciar" na segunda linha do `buttons`. Verificado por pixel com o cursor real em cima (tema escuro, 100%): o fundo vai de 33 para 46 em toda a área do botão, inclusive no anel da borda, sem emenda. Bate com `#0FFFFFFF` sobre 33 (33 + 0.059 × 222 ≈ 46).

### ToggleButton

`Button::toggle(label, rect, checked)` é o `ToggleButton` de dois estados (os blocos de Configurações rápidas do Win11 são desse tipo). Usa a mesma entrada do `ButtonBase`; `is_checked()` devolve `None` num botão comum.

| Regra | Fonte |
|---|---|
| Cada clique (ponteiro, teclado ou `invoke()`) inverte `IsChecked` antes de reportar o clique | `ToggleButton::OnClick` → `OnToggle` (`ToggleButton_Partial.cpp`) |
| `invoke()` é o `OnClick` programático, usado pelo UI Automation; desabilitado, não faz nada | `AutomationToggleButtonOnToggle` chama `OnClick`; o peer recusa se `!IsEnabled` |
| Estados visuais: Checked/CheckedPointerOver/CheckedPressed/CheckedDisabled, mesma prioridade do Button | `ToggleButton::ChangeVisualState` |
| Fora do alto contraste, Checked* = pincéis do accent (`AccentFillColor*`, `TextOnAccentFillColor*`, `AccentControlElevationBorderBrush`, sem borda no Pressed/Disabled). Fundo `OuterBorderEdge` em Checked/CheckedPointerOver/CheckedPressed; CheckedDisabled não define e fica `InnerBorderEdge` (anel transparente de 1 DIP) | [`ToggleButton_themeresources.xaml`](https://github.com/microsoft/microsoft-ui-xaml/blob/main/controls/dev/CommonStyles/ToggleButton_themeresources.xaml) (`ToggleButtonCheckedStateBackgroundSizing`) |
| Alto contraste, Checked: Highlight / HighlightText / HighlightText; Over: ButtonText / ButtonFace / ButtonText; Pressed: HighlightText / Highlight / HighlightText; Disabled igual ao Button | idem, dicionário `HighContrast` + `generic.xaml` |
| Marcar/desmarcar também anima o fundo (83 ms) | `BrushTransition` do template |

Exemplo: o `buttons` tem um "Wi-Fi" (começa ligado) numa segunda linha. Ao clicar dispara `ToggleState` alterado (`UiaRaiseAutomationPropertyChangedEvent`), não `Invoke_Invoked`, porque o peer do WinUI não tem Invoke. Verificado com cliente UIA real (`System.Windows.Automation`): tipo "botão", só `TogglePattern`, `Toggle()` três vezes dá On → Off → On → Off, e chegam os eventos `1->0`, `0->1`, `1->0`. Ligado aparece com o visual do accent. 51.6 MB com UIA ativo, código de saída 0.

### Alto contraste

Com alto contraste ligado, `Theme::system()` devolve `Theme::high_contrast(SystemColors::current())`. O campo `Theme::high_contrast` guarda as cores do usuário e o `Button` troca a tabela de tokens pelo dicionário `HighContrast` do WinUI. Toda borda vira sólida de 1 DIP, sem gradiente de elevação.

| Estado | Padrão: fundo / texto / borda | Accent: fundo / texto / borda |
|---|---|---|
| Rest | ButtonFace / ButtonText / ButtonText | Highlight / HighlightText / Highlight |
| PointerOver | HighlightText / Highlight / Highlight | ButtonText / ButtonFace / Highlight |
| Pressed | HighlightText / Highlight / HighlightText | ButtonFace / Highlight / Highlight |
| Disabled | ButtonFace / GrayText / GrayText | ButtonFace / GrayText / GrayText |

| Decisão | Fonte |
|---|---|
| Detecção: `SystemParametersInfoW(SPI_GETHIGHCONTRAST)` com `HCF_HIGHCONTRASTON`; cores por `GetSysColor` (`COLOR_WINDOW`, `WINDOWTEXT`, `BTNFACE`, `BTNTEXT`, `HIGHLIGHT`, `HIGHLIGHTTEXT`, `GRAYTEXT`) | [High contrast parameter](https://learn.microsoft.com/windows/win32/winauto/high-contrast-parameter) |
| Tabela acima: dicionário `HighContrast` do Button, com cada `SystemControl*Brush` resolvido pelo `generic.xaml` do framework (por exemplo `SystemControlHighlightBaseHighBrush` = Highlight) | [`Button_themeresources.xaml`](https://github.com/microsoft/microsoft-ui-xaml/blob/main/controls/dev/CommonStyles/Button_themeresources.xaml), [`generic.xaml`](https://github.com/microsoft/microsoft-ui-xaml/blob/main/dxaml/xcp/dxaml/themes/generic.xaml) |
| Tokens comuns (texto = WindowText, desabilitado = GrayText, cartão = Window, borda = ButtonText) e foco: externo WindowText, interno Window | `Common_themeresources_any.xaml` e `Common_themeresources.xaml` (`SystemControlFocusVisual*` → `FocusStrokeColor*`), dicionário `HighContrast` |
| `Theme::dark` no alto contraste = fundo Window escuro (`IsColorLight`) | Inferência: decide o modo escuro do DWM na barra de título |
| Exemplo: com alto contraste, pinta a área cliente com Window (opaca, sem Mica) e relê o tema em `WM_SYSCOLORCHANGE` e em `WM_SETTINGCHANGE` com `SPI_SETHIGHCONTRAST` ou `ImmersiveColorSet` | `SolidBackgroundFillColorBase` = Window no dicionário `HighContrast`; [High contrast parameter](https://learn.microsoft.com/windows/win32/winauto/high-contrast-parameter) |

Verificado: desenho com uma paleta HC forçada (preto, branco, Highlight lilás), igual à tabela. Verificado ao vivo: com o `buttons` aberto sem alto contraste, ligar o modo (Alt+Shift+Print Screen) redesenhou a janela com as cores do tema do usuário (ButtonText amarelo, Highlight lilás, fundo Window preto), sem reiniciar. 48.6 MB, código de saída 0. Não verificado: o `demo` ao vivo e a volta ao tema normal.

Pendente: `AnimatedIcon`.

Não citado literalmente: o corpo de `OnPointerPressed`/`OnPointerReleased`. As regras de press e release vêm do padrão confirmado em `OnPointerMoved` e `KeyUp`.

## CheckBox

`check_box::CheckBox::new(label, rect, checked)` é o `CheckBox` de dois estados do WinUI (`DefaultCheckBoxStyle`). A entrada é a do ToggleButton, porque o `CheckBox` deriva dele.

| Regra | Fonte |
|---|---|
| Enter não faz nada; só Espaço (e clique) alterna | `CheckBox::Initialize` → `SetAcceptsReturn(false)`; `ButtonBaseKeyProcess.h` (`IsPress`). O `Button` ganhou `accepts_return` (interno ao crate) |
| `+` do teclado numérico marca, `-` desmarca, cancelando um Espaço segurado (`put_IsPressed(FALSE)`); desabilitado ignora | `CheckBox::OnKeyDownInternal` (`VirtualKey_Add/Subtract`). `Key::Add`/`Key::Subtract` novos; no Button cancelam um Espaço segurado como qualquer tecla que não pressiona |
| Quadrado 20×20 raio 4, borda 1, centralizado na vertical; texto 8 DIPs à direita e 5 do topo; mínimo 120×32; foco a `-7,-3,-7,-3` do controle todo | `CheckBoxSize`, `CheckBoxPadding`, `CheckBoxMinWidth`, `CheckBoxHeight`, `CheckBoxFocusVisualMargin` |
| Glifo: `E73E` (CheckMark) da Segoe Fluent Icons em 12 DIPs, 1 DIP abaixo do centro, só quando marcado | O template usa `AnimatedIcon` (Lottie `AnimatedAcceptVisualSource`) com `FontIconSource` `CheckBoxCheckedGlyph` em `SymbolThemeFontFamily` como fallback. Sem Lottie, usamos o fallback; como ele não tem quadro "desligado", o glifo só é desenhado marcado. `Surface::icon_format` novo |
| Pincéis: desmarcado `ControlAltFillColor*` + `ControlStrongStrokeColor*` (Pressed usa o stroke Disabled); marcado acento ×1/0.9/0.8 no fundo e na borda, glifo `TextOnAccentFillColor*`; todas as trocas são discretas (sem transição) | `CheckBox_themeresources.xaml` |
| Alto contraste: tabela própria e fundo do controle todo em Window (`CheckBoxBackground*`) | dicionário `HighContrast` |

`CheckBox::three_state(label, rect, CheckState)` liga o `IsThreeState`: cliques fazem Unchecked → Checked → Indeterminate → Unchecked (`ToggleButton::OnToggleImpl`); `state()` devolve `CheckState`. Fora do alto contraste o indeterminado usa os pincéis do marcado (o XAML aponta as chaves `Indeterminate*` para os mesmos tokens); no alto contraste tem tabela própria, com texto em WindowText. Glifo `E9AE` (`CheckBoxIndeterminateGlyph`) sem a margem de 1 DIP (os estados Indeterminate zeram `CheckGlyph.Margin`). Com três estados o `+`/`-` não faz nada (`OnKeyDownInternal` exige `!IsThreeState`).

Exemplo: "Lembrar de mim" (começa marcado) ao lado do switch; o `enum Control` ganhou `Check` e um `Formats` com os quatro formatos de texto. Verificado:
- Cliente UIA real: tipo "caixa de seleção", só `TogglePattern`, `Toggle()` dá On → Off → On → Off com os eventos `1->0`, `0->1`, `1->0`.
- Teclado (mensagens postadas com o foco no CheckBox via UIA): `-` desmarca, `-` de novo não muda, `+` marca, Enter não faz nada, Espaço alterna.
- Três estados ("Selecionar tudo", começa indeterminado): `Toggle()` dá Indeterminate → Off → On → Indeterminate com os eventos `2->0`, `0->1`, `1->2`, e o traço `E9AE` aparece no quadrado.
- Tela: quadrado no acento com o check escuro (glifo da Segoe Fluent Icons) quando marcado; quadrado vazio com borda quando desmarcado. 51.5 MB com UIA ativo, código de saída 0.

## ToggleSwitch

`toggle_switch::ToggleSwitch::new(header, on_content, off_content, rect, on)` reproduz o `ToggleSwitch` do WinUI (`DefaultToggleSwitchStyle`). Usa o mesmo `Input`/`Response`/`Visual` do Button; `Response::clicked` = `IsOn` mudou (`Toggled`). `desired_size` mede header + conteúdo.

| Regra | Fonte |
|---|---|
| Geometria: header (se houver) + 4, depois linha de 40 (10 + trilho 40×20 raio 10 + 10); conteúdo 12 DIPs à direita do trilho; largura mínima 154 | `ToggleSwitch_themeresources.xaml` (`OuterBorder`, `ToggleSwitchPre/PostContentMargin` = 10, `ToggleSwitchThemeMinWidth`) |
| Knob 12 (Rest/Disabled), 14 (PointerOver), 17×14 (Pressed, ancorado à esquerda desligado e à direita ligado, a 3 DIPs), raio 7; tamanho anima 83 ms `0,0,0,1` (250 ms ao desabilitar) | idem (estados `Normal`/`PointerOver`/`Pressed`/`Disabled`) |
| Ligado: knob desloca 20 DIPs (`KnobTranslateTransform`) | idem (estado `On`) |
| Clique (tap) alterna. O press já inicia o drag do `Thumb` (visual Pressed); arrastar ≥ 4 DIPs move o knob junto e, ao soltar, alterna se passou da metade | `ToggleSwitch_Partial.cpp` (`DragStarted/Delta/Completed`, `MoveCompleted`, `TapHandler`). Limiar de 4 DIPs = `SM_CXDRAG` padrão (simplificação) |
| Teclado: só Espaço, alternando no KeyUp se o KeyDown foi tratado. Enter não faz nada. O ramo de setas/Home/End do `ToggleSwitchKeyProcess.h` nunca roda porque `HandlesKey` só aceita Space/GamepadA | `ToggleSwitchKeyProcess.h`, `ToggleSwitch::HandlesKey` |
| Alternar desliza o knob em 367 ms com `cubic-bezier(0.1, 0.9, 0.2, 1)`; Off→On esmaece o visual ligado em 83 ms, On→Off troca na hora; soltar um drag (ou perder a captura) desliza o knob do ponto onde estava até o fim, com a mesma animação, e esmaece em 83 ms. Um `Toggle()` via UIA durante o drag só muda `IsOn`; quem decide o visual é a soltura | `RepositionThemeAnimation` no `OnToOff`/`OffToOnTransition`; tempo e curva lidos do uxtheme deste Windows: classe `ANIMATIONS`, `TAS_REPOSITION` (= 3, `vsanimation.h`) → 367 ms, função de tempo 4 da classe `timingfunction` → (0.1, 0.9, 0.2, 1.0) |
| Cores do trilho desligado animam 83 ms para PointerOver/Pressed/Disabled e trocam na hora para Normal; o resto é discreto | `ColorAnimationUsingKeyFrames` vs `DiscreteObjectKeyFrame` no template |
| Pincéis: desligado `ControlAltFillColor` Secondary/Tertiary/Quarternary/Disabled + `ControlStrongStrokeColor`; ligado acento ×1/0.9/0.8/disabled, knob `TextOnAccentFillColorPrimary`; knob desligado `TextFillColorSecondary`; texto Primary/Disabled | idem + `Common_themeresources_any.xaml` (tokens novos no `Theme`) |
| Alto contraste: tabela própria (`SystemColor*`) e trilho ligado com borda de 1 DIP (`ToggleSwitchOnStrokeThickness` = 1 só nesse dicionário) | dicionário `HighContrast` |
| Foco: alvo `SwitchAreaGrid` (linha recuada 5 DIPs), anel a `-7,-3,-7,-3` | `IsTemplateFocusTarget`, `FocusVisualMargin` |
| Área de clique e de foco = trilho + 12 + conteúdo mais largo (`ToggleSwitch::layout` mede); o resto até a largura mínima de 154 não reage | O Grid interno é `HorizontalAlignment=Left` com largura Auto; o Grid raiz não tem fundo e o header é `IsHitTestVisible=False` |
| `Toggle()` durante o drag não mexe no knob | `OnToggledImpl` pula `UpdateVisualState` com `m_isDragging` |

UI Automation: no modelo vira um item com `toggle = Some(IsOn)` e Name = header, o que expõe tipo Button com só o padrão Toggle, como o `ToggleSwitchAutomationPeer` (que também devolve `AutomationControlType_Button` e monta o Name do header).

Exemplo: "Modo avião" (Ativado/Desativado) numa terceira linha do `buttons`, que agora guarda os controles num `enum Control { Button, Switch }`. Verificado:
- Cliente UIA real: tipo "botão", só `TogglePattern`, `Toggle()` dá Off → On → Off → On com os eventos `0->1`, `1->0`, `0->1`.
- Mouse real (`SendInput`, 100%): tap liga; arraste de 6 px não alterna; arraste de 16 px alterna para os dois lados.
- Tela: trilho ligado no acento, knob branco à direita, anel de foco em volta da área do switch. 51.8 MB com UIA ativo, código de saída 0.

Simplificações: durante o drag as cores ficam as do estado atual (no WinUI o estado `Dragging` também mantém o visual do lado de partida); sem `HeaderPlacement=Left`; o raio do anel de foco usa o mesmo ajuste do Button (+3).

## Slider

`slider::Slider::new(name, rect, minimum, maximum, value)` é o `Slider` horizontal do WinUI (`DefaultSliderStyle`), passo 1. É o controle de volume e brilho das Configurações rápidas.

| Regra | Fonte |
|---|---|
| Altura 32: 14 + trilho 4 (raio 2) + 14; thumb 18 com anel visível de 22 (`Margin=-2`, raio 10) e elipse interna de 12 | `Slider_themeresources.xaml` (`SliderPre/PostContentMargin`, `SliderTrackThemeHeight`, `SliderHorizontalThumb*`, `SliderInnerThumb*`) |
| Posição do thumb = fração × (largura − 18); o preenchimento de valor vai do início até a borda esquerda do thumb | `Slider::UpdateTrackLayout` (`multiplier * (actualWidth - thumbWidth)`) |
| Press no trilho: captura, foco tipo ponteiro, thumb vai para o ponteiro (centro sob o cursor) e segue o mouse até soltar; press no thumb arrasta relativo, sem pulo | `OnPointerPressed` → `MoveThumbToPoint`; o `Thumb` trata o próprio press (`DragDelta`) |
| Valor arredondado ao passo e limitado a Minimum..Maximum | `GetClosestStep` (`SliderSnapsTo_StepValues`) |
| Setas ±1 (esquerda/baixo diminui), Home/End vão aos extremos; sem PageUp/PageDown | `SliderKeyProcess.h` |
| Elipse interna escala 0,86 (Normal, 167 ms), 1,167 (PointerOver, 250 ms), 0,71 (Pressed, 250 ms), 1,167 (Disabled, 167 ms), curva `0,0,0,1` | `SliderThumbStyle` |
| Pincéis: trilho `ControlStrongFillColor` Default/Disabled; valor e thumb acento ×1/0.9/0.8/disabled; anel `ControlSolidFillColorDefault` com borda de elevação | idem (tokens novos no `Theme`) |
| Alto contraste: trilho 2 DIPs ButtonText, valor Highlight, thumb ButtonText/Highlight/HotLight/GrayText (`COLOR_HOTLIGHT` novo em `SystemColors`), anel ButtonText com borda sólida | dicionário `HighContrast` + `generic.xaml` |
| Foco: o controle todo a `-7,0,-7,0` | `FocusVisualMargin` |

Simplificações: sem header, sem marcas (`TickBar`), sem tooltip de valor, só horizontal; perder o foco encerra um press (o XAML mantém a captura); o trilho de 2 DIPs do alto contraste fica centrado na linha do thumb (no XAML fica 1 DIP acima).

## Panel (roteamento comum)

`panel::Panel` guarda os controles em ordem de Tab e faz o roteamento que toda janela precisa; antes isso vivia copiado no exemplo.

| Regra | Fonte |
|---|---|
| Movimento/soltura/saída do ponteiro vão só para o controle com a captura, como o XAML roteia eventos de ponteiro capturado; teclas vão para o controle com foco | `ButtonBase` + roteamento de ponteiro do XAML |
| Press que pede foco tira o foco do anterior (que solta a captura se a tinha: Tab com o mouse apertado) | `OnLostFocus` |
| Tab/Shift+Tab pulam controles desabilitados | `IsTabStop` em controle desabilitado |
| `Outcome { redraw, capture, events }`: a janela chama `SetCapture`/`ReleaseCapture` (vale o último pedido), dispara os eventos UIA e repinta | — |
| Eventos: `Invoked(i)` para botão comum, `Toggled { index, old, new }` para quem tem o padrão Toggle (estado antes e depois do clique), `Focused(i)` quando o foco muda | Peers do WinUI: só Invoke ou só Toggle |
| `trait Control` (Button, ToggleSwitch, CheckBox) e `Formats` (os quatro formatos de texto); `uia_items()` monta o modelo do `uia` | — |

## UI Automation

`uia` expõe controles desenhados à mão para leitores de tela (Narrador, NVDA) e ferramentas de teste. A janela devolve uma raiz de fragmento no `WM_GETOBJECT` (`UiaRootObjectId`), e cada controle é um fragmento filho do tipo Button com o padrão Invoke.

| Decisão | Motivo | Fonte |
|---|---|---|
| Providers leem um `Model` compartilhado (`Rc<RefCell>`) que o app atualiza | Uma chamada do UIA nunca disputa o borrow do app | — |
| `ProviderOptions_ServerSideProvider \| UseComThreading`, thread de UI em STA | As chamadas chegam na própria thread de UI, sem lock | [ProviderOptions](https://learn.microsoft.com/windows/win32/api/uiautomationcore/ne-uiautomationcore-provideroptions) |
| `Invoke` e `SetFocus` postam `WM_UIA_INVOKE`/`WM_UIA_FOCUS` | `Invoke` precisa voltar na hora, sem bloquear | [IInvokeProvider::Invoke](https://learn.microsoft.com/windows/win32/api/uiautomationcore/nf-uiautomationcore-iinvokeprovider-invoke) |
| Raiz: host via `UiaHostProviderFromHwnd`, runtime id NULL, só navega para os filhos | O provider da janela cuida de pai, irmãos e nome | [Navigate](https://learn.microsoft.com/windows/win32/api/uiautomationcore/nf-uiautomationcore-irawelementproviderfragment-navigate), [GetRuntimeId](https://learn.microsoft.com/windows/win32/api/uiautomationcore/nf-uiautomationcore-irawelementproviderfragment-getruntimeid) |
| Filho: Button, Name, AutomationId único (`button-N`), IsEnabled, IsKeyboardFocusable, HasKeyboardFocus, IsControl/ContentElement | Requisitos do tipo Button | [Button control type](https://learn.microsoft.com/windows/win32/winauto/uiauto-supportbuttoncontroltype) |
| `BoundingRectangle` em pixels de tela, sem recorte | Exigido pela API | [BoundingRectangle](https://learn.microsoft.com/windows/win32/api/uiautomationcore/nf-uiautomationcore-irawelementproviderfragment-get_boundingrectangle) |
| Invoke/SetFocus desabilitado → `UIA_E_ELEMENTNOTENABLED`; controle removido → `UIA_E_ELEMENTNOTAVAILABLE` | A página de `Invoke` só diz "HRESULT"; o código vem da tabela de erros do UIA | [UIA error codes](https://learn.microsoft.com/windows/win32/winauto/uiauto-error-codes) |
| Toggle button (`Item::toggle = Some`): tipo Button com só o padrão Toggle (sem Invoke); `ToggleState` On/Off do modelo; `Toggle()` desabilitado → `UIA_E_ELEMENTNOTENABLED` | É o que o `ToggleButtonAutomationPeer` faz: só `PatternInterface_Toggle`, e o `ButtonBaseAutomationPeer` não expõe Invoke | `ToggleButtonAutomationPeer_Partial.cpp`, `ButtonAutomationPeer_Partial.cpp`, [IToggleProvider](https://learn.microsoft.com/windows/win32/api/uiautomationcore/nn-uiautomationcore-itoggleprovider) |
| `Toggle()` posta `WM_UIA_INVOKE`, como o Invoke | O provider não pode mexer no botão (o app é dono). Limite: logo depois do `Toggle()`, o `ToggleState` ainda é o antigo até o app tratar a mensagem; o evento de propriedade avisa a troca. No WinUI o `OnClick` é síncrono | Decisão própria |
| `Item::toggle` é `Option<ToggleState>` (On, Off ou Indeterminate) e o evento de troca leva o valor antigo e o novo | CheckBox de três estados; o exemplo lê o valor antigo do modelo antes de sincronizar | [ToggleState](https://learn.microsoft.com/windows/win32/api/uiautomationcore/ne-uiautomationcore-togglestate) |
| `Item::control_type` (Button ou CheckBox) | O `CheckBoxAutomationPeer` devolve `AutomationControlType_CheckBox`; Button, ToggleButton e ToggleSwitch devolvem Button | `CheckBoxAutomationPeer_Partial.cpp` |
| Dependência direta `windows-core` 0.62.2 | O `#[implement]` gera caminhos `::windows_core`; já estava na árvore | `Cargo.lock` |

Testes: 22 com objetos COM reais numa janela oculta (navegação, propriedades, tipo de controle, pixels de tela, hit test, foco, runtime ids, Invoke assíncrono e desabilitado, Toggle no lugar de Invoke, ToggleState, Toggle assíncrono e desabilitado, elemento removido).

Integração (`examples/buttons.rs`): thread de UI em STA (`CoInitializeEx(COINIT_APARTMENTTHREADED)`); `WM_GETOBJECT` com `UiaRootObjectId` devolve a raiz; o modelo é sincronizado a cada render; eventos `AutomationFocusChanged` e `Invoke_Invoked` (só quando `UiaClientsAreListening`); `UiaReturnRawElementProvider(hwnd, 0, 0, NULL)` no `WM_DESTROY`.

Verificado com um cliente UIA real (`System.Windows.Automation`): 4 botões encontrados, com tipo localizado "botão", ids únicos, IsEnabled/IsKeyboardFocusable corretos e retângulos em pixels de tela. `FromPoint` acerta o botão, `SetFocus` move o foco, Invoke clica (a tela mostra "Clicou: Salvar") e Invoke em botão desabilitado dá `ElementNotEnabledException`. Com UIA ativo: 51.0 MB, 18 threads (o runtime do UIA cria threads próprias), 0 ms de CPU parado.

Robustez (revisão Opus):
- Os eventos UIA entram numa fila e são disparados depois que o borrow do app termina, porque disparar pode bombear mensagens da STA. `WM_UIA_*` que chegam com o app ocupado são postados de novo, em vez de se perderem.
- `WM_UIA_FOCUS` só move o foco se a janela tiver o foco do teclado (`GetFocus() == hwnd`).
- O `WM_DESTROY` esvazia o modelo, então quem ainda segura um provider recebe `UIA_E_ELEMENTNOTAVAILABLE`, e não retângulos em (0,0) ou mensagens postadas para um HWND morto.
- `CoUninitialize` via guarda de drop em todos os caminhos de saída.

Verificado com um listener C# sobre `System.Windows.Automation`: chegam os eventos `Focus:Configurações`, `Focus:Salvar` e `Invoked:Salvar`.

Observado e sem documentação: o cliente .NET chama `SetFocus` no botão antes do `Invoke`, então o foco vai para o botão invocado. O provider obedece o pedido. Não está confirmado se quem pede é o cliente gerenciado ou o UIA core.

Eventos de propriedade: `ToggleState` (antigo → novo) e `BoundingRectangle` depois de `WM_DPICHANGED` (o retângulo em pixels de tela muda; `uia::bounds_variant` monta o `VT_ARRAY | VT_R8` com left, top, width, height; valor antigo `VT_EMPTY`, porque o retângulo anterior não é guardado). Mover a janela (ou redimensioná-la pela borda esquerda/superior) muda o `BoundingRectangle` de todos os controles em pixels de tela, mas não dispara evento: limitação deliberada, para não gerar uma rajada a cada `WM_MOVE` durante o arraste. Os clientes acompanham a janela pelos eventos de localização do próprio HWND e releem os retângulos quando precisam (inferência; não verificado com Narrador/NVDA). `IsEnabled` não tem evento porque o exemplo não muda isso em tempo de execução; quem mudar deve disparar `UIA_IsEnabledPropertyId`. Verificado com troca real de escala (125% → 150% com o exemplo aberto): chegaram 9 eventos de `BoundingRectangle`, um por controle, com o retângulo novo (Salvar 91×48 px = 32 DIPs × 1,5). Narrador: funcionou no teste manual do usuário (Tab e Espaço em todos os controles). NVDA não testado.

## Tipografia

Altura de linha natural medida pelo teste `every_ramp_step_should_have_a_natural_line_taller_than_its_size` (`cargo test -- --nocapture`), comparada com a especificação de design:

| Estilo | Tamanho | Peso | Linha natural (DirectWrite) | Linha no design | Baseline |
|---|---|---|---|---|---|
| Caption | 12 | 400 | 15.96 | 16 | 12.95 |
| Body | 14 | 400 | 18.62 | 20 | 15.11 |
| Body Strong | 14 | 600 | 18.62 | 20 | 15.11 |
| Body Large | 18 | 400 | 23.94 | 24 | 19.42 |
| Subtitle | 20 | 600 | 26.60 | 28 | 21.58 |
| Title | 28 | 600 | 37.24 | 36 | 30.21 |
| Title Large | 40 | 600 | 53.20 | 52 | 43.16 |
| Display | 68 | 600 | 90.45 | 92 | 73.38 |

Não verificado: a comparação pixel a pixel com um `TextBlock` do WinUI renderizado. O Learn não documenta onde o XAML posiciona a baseline dentro da linha.

## Medições

Demo em release, Windows 11 (build 26300), janela de 4 tiles com Mica:

| | fluent-win demo | explorer.exe |
|---|---|---|
| Working set | 46.7 MB | 327.7 MB |
| CPU parado (3 s, também depois de minimizar/restaurar) | 0 ms | — |
| Threads | 14 | — |
| Tamanho do exe | 151 KB | — |

A maior parte da RAM é o custo fixo do driver D3D11 carregado no processo. `UISettings` (WinRT) e `IDWriteFactory6` somaram cerca de 1 MB em relação à versão com registro (45.7 MB).

## Pendências

- [x] Line height do type ramp: resolvido seguindo o WinUI, que não força `LineHeight` (ver Tipografia)
- [x] Acento e tema por API documentada (`UISettings`)
- [x] Recuperar device perdido (reset de GPU, troca de driver, RDP)
- [x] Button: UI Automation (fragmentos, Invoke, foco), verificado com cliente UIA real
- [x] Button: alto contraste (tabela `HighContrast` do WinUI; os dois exemplos pintam fundo Window e recarregam o tema ao vivo)
- [x] Alto contraste ligado de verdade com o `buttons` aberto (troca ao vivo)
- [x] Escalas fracionárias: 125% e 150% conferidas com zoom nas bordas (sem emenda nem borrão), e troca ao vivo 125% → 150% redimensiona a janela para 780×450 (520×300 DIPs) e escala o conteúdo
- [x] Eventos UIA de BoundingRectangle (troca de DPI) e ToggleState; IsEnabled não muda em tempo de execução no exemplo
- [x] Narrador: com o `buttons` aberto, Tab por todos os controles e Espaço nos de alternar funcionaram (relato do usuário, 2026-11-22; sem transcrição da fala). NVDA não testado
- [x] Texto sem alocação no heap por frame: rótulos de até 128 unidades UTF-16 são codificados na pilha (`with_wide`); só textos maiores alocam
- Fora do escopo: Mica no Win10/Win11 21H2 (a meta é o Win11 atual)

## Licença

MIT OR Apache-2.0
