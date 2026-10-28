# TrayTemp

Temperatura da CPU (por núcleo) e da GPU na bandeja do Windows, ao lado do relógio.
Um único `.exe` nativo (~300 KB, ~2 MB de memória privada), sem runtime.

## Recursos

- **CPU por núcleo**: lê o sensor digital de cada núcleo físico (P-cores e E-cores separados) e o do pacote.
- **Na bandeja**, escolha pelo menu: temperatura **máxima** entre os núcleos, **média**, **pacote**, ou **um ícone por núcleo**.
- **GPU dedicada** (NVIDIA/AMD) pela mesma interface do Gerenciador de Tarefas. Em notebooks, se a GPU
  estiver desligada (D3), ela não é consultada, para não ser acordada; o ícone mostra `--`.
- **Clique esquerdo**: painel com atual / mínima / máxima de cada núcleo e da GPU.
- **Clique direito**: menu com modo de exibição, intervalo (1/2/5 s), zerar mín/máx e *Iniciar com o Windows*.
- Cores: branco (CPU) e verde (GPU); laranja a partir de 85 °C e vermelho a partir de 95 °C. Segue o tema claro/escuro.

## Requisitos

- CPU Intel com sensor térmico digital (Core 2 em diante).
- Driver **PawnIO** (assinado, open source, o mesmo usado pelo LibreHardwareMonitor):
  `winget install namazso.PawnIO` ou https://pawnio.eu
- O app pede elevação (UAC), porque o PawnIO só atende processos de administrador.

## Inicialização com o Windows

Use *Iniciar com o Windows* no menu. Isso cria a tarefa agendada `TrayTemp`, que roda no logon com
privilégios elevados **sem mostrar o UAC** e sem as restrições padrão de bateria do Agendador.
A tarefa aponta para o caminho atual do `.exe`; se movê-lo, desmarque e marque a opção de novo.

## Configuração

Salva em `%APPDATA%\TrayTemp\config.ini` (o menu cuida da maior parte):

```ini
cpu_mode=max        ; max | avg | package | cores
show_gpu=1
interval_ms=2000
warn=85             ; °C para laranja
crit=95             ; °C para vermelho
font=Trebuchet MS   ; fonte dos números na bandeja
font_size=8         ; em pontos (0 = números preenchem o ícone inteiro)
font_bold=0
```

## Compilar

Rust (toolchain MSVC):

```
cargo build --release
```

Diagnóstico sem interface (grava as leituras num arquivo):

```
TrayTemp.exe --dump C:\caminho\saida.txt
```

Prévia dos ícones renderizados (não precisa de elevação):

```
set __COMPAT_LAYER=RunAsInvoker
set TRAYTEMP_PREVIEW=%TEMP%\preview.bmp
cargo test --release preview -- --ignored
```

## Créditos

`assets/IntelMSR.bin` é um módulo do [PawnIO.Modules](https://github.com/namazso/PawnIO.Modules)
(release 0.2.11, LGPL-2.1, ver `assets/PawnIO.Modules-COPYING.txt`).
