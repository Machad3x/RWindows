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
let surface = Surface::new(hwnd)?;
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
| Durações 83/167/250 ms, curva `0,0,0,1` | `ControlFaster/Fast/NormalAnimationDuration`, `ControlFastOutSlowInKeySpline` | WinUI `Common_themeresources_any.xaml` |
| Idle bloqueado em `GetMessageW` e frames só com animação ativa | 0% de CPU parado | Medido |
| Fonte: família tipográfica `Segoe UI Variable` via `IDWriteFactory6`, com `DWRITE_AUTOMATIC_FONT_AXES_OPTICAL_SIZE` e peso pelo eixo `wght` | É o que o XAML faz: `XamlAutoFontFamily` e tamanho óptico automático. Os nomes WWS ("… Small/Text/Display") eram uma aproximação | [Typography in Windows](https://learn.microsoft.com/windows/apps/design/signature-experiences/typography), [DWRITE_AUTOMATIC_FONT_AXES](https://learn.microsoft.com/windows/win32/api/dwrite_3/ne-dwrite_3-dwrite_automatic_font_axes), [CreateTextFormat (Factory6)](https://learn.microsoft.com/windows/win32/api/dwrite_3/nf-dwrite_3-idwritefactory6-createtextformat) |
| Tema e acento via `UISettings.GetColorValue` (WinRT), não pelo registro | API documentada e a mesma fonte que o WinUI usa. `AccentPalette` no registro não tem documentação. Modo escuro = foreground claro (`IsColorLight`). Acento: `AccentLight2` no escuro, `AccentDark1` no claro (`AccentFillColorDefault`). O windows-core entra no MTA sozinho quando recebe `CO_E_NOTINITIALIZED` | [Apply Windows themes](https://learn.microsoft.com/windows/apps/desktop/modernize/ui/apply-windows-themes), [UIColorType](https://learn.microsoft.com/uwp/api/windows.ui.viewmanagement.uicolortype), [Theming: accent palette](https://learn.microsoft.com/windows/apps/develop/ui/theming#accent-color-palette), WinUI `Common_themeresources_any.xaml` |
| Constantes `DARK.accent` #60CDFF / `LIGHT.accent` #005FB8 | Só usadas se o WinRT falhar. São os valores de design do WinUI. O Learn não publica as variações do azul padrão do Win11 | Não verificado |
| **Sem** `LineHeight` forçado: a linha usa as métricas naturais da fonte | Os estilos do WinUI não definem `LineHeight` (só `LineStackingStrategy=MaxHeight`), então o Win11 real usa a altura natural. Os valores 16/20/28… são da especificação de design | [`TextBlock_themeresources.xaml`](https://github.com/microsoft/microsoft-ui-xaml/blob/main/controls/dev/CommonStyles/TextBlock_themeresources.xaml), [LineStackingStrategy](https://learn.microsoft.com/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.linestackingstrategy) |

## Tipografia

Altura de linha natural medida pelo teste `every_ramp_step_should_have_a_natural_line_taller_than_its_size` (`cargo test -- --nocapture`), comparada com a especificação de design:

| Estilo | Tamanho | Peso | Linha natural (WinUI real) | Linha no design | Baseline |
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
| Working set | 45.7 MB | 327.7 MB |
| CPU parado | 0% | — |
| Threads | 14 | — |
| Tamanho do exe | 146 KB | — |

A maior parte da RAM é o custo fixo do driver D3D11 carregado no processo.

## Pendências

- [x] Line height do type ramp: resolvido seguindo o WinUI, que não força `LineHeight` (ver Tipografia)
- [x] Acento e tema por API documentada (`UISettings`)
- [ ] Recuperar device perdido (reset de GPU, troca de driver, RDP)
- [ ] Uma alocação UTF-16 por texto por frame
- Fora do escopo: Mica no Win10/Win11 21H2 (a meta é o Win11 atual)

## Licença

MIT OR Apache-2.0
