# BUTION

BUTION объединяет несколько компьютеров в локальной сети для запуска одной
GGUF-модели через `llama.cpp RPC`.

Поддерживаются:

- macOS на Apple Silicon и Intel;
- Windows x64.

## Установка

### macOS

Откройте Terminal и выполните:

```bash
curl -fsSL https://raw.githubusercontent.com/danytebyya/bution/main/install.sh | bash
```

### Windows

Откройте обычный PowerShell и выполните:

```powershell
irm https://raw.githubusercontent.com/danytebyya/bution/main/install.ps1 | iex
```

Установщик скачает BUTION и официальный `llama.cpp`, добавит команду `bution` в
PATH и настроит необходимые правила Windows Firewall. Если готового Release нет,
он автоматически установит недостающие инструменты и соберёт BUTION из исходников.

После установки откройте новое окно терминала.

## Быстрый старт

1. Подключите все компьютеры к одной локальной сети.
2. Установите и запустите BUTION на каждом дополнительном компьютере:

   ```text
   bution
   ```

3. На основном компьютере, где хранится GGUF-модель, запустите:

   ```text
   bution --model "/полный/путь/к/model.gguf"
   ```

4. Проверьте имя найденного узла и подтвердите pairing на обоих компьютерах.
5. Дождитесь проверки сети на экране `Benchmark`.
6. На основном компьютере откройте `Cluster` и нажмите `Enter`.
7. После запуска модели перейдите в `Chat`.

Модель нужна только на основном компьютере. На дополнительные узлы копировать
GGUF-файл не требуется.

## Повторный запуск установщика

Обычный повторный запуск безопасен: уже установленные BUTION, `llama.cpp`, Rust,
C++ Build Tools, PATH и правила Firewall не загружаются и не создаются повторно.
Если установка ранее оборвалась, будут добавлены только отсутствующие компоненты.

Для принудительного обновления BUTION и `llama.cpp`:

macOS:

```bash
curl -fsSL https://raw.githubusercontent.com/danytebyya/bution/main/install.sh | BUTION_FORCE_UPDATE=1 bash
```

Windows:

```powershell
$env:BUTION_FORCE_UPDATE="1"; irm https://raw.githubusercontent.com/danytebyya/bution/main/install.ps1 | iex
```

## Прямое Ethernet-соединение

Можно оставить Wi-Fi для интернета и соединить два компьютера Ethernet-кабелем.
BUTION проверит доступные маршруты и выберет более быстрый.

Если адреса не назначились автоматически:

- первый компьютер: `192.168.50.1`, маска `255.255.255.0`;
- второй компьютер: `192.168.50.2`, маска `255.255.255.0`;
- gateway и DNS для Ethernet оставить пустыми.

## Как опубликовать Release

Release рекомендуется: пользователи получат готовый бинарник и им не понадобятся
Rust или C++ Build Tools.

Через интерфейс GitHub:

1. Откройте вкладку `Actions` репозитория.
2. Выберите workflow `Release binaries`.
3. Нажмите `Run workflow`.
4. Укажите новую версию, например `v0.1.0`, и подтвердите запуск.
5. После завершения проверьте вкладку `Releases` — там должны появиться три архива.

Альтернативный запуск из терминала:

```bash
git tag -a v0.1.0 -m "BUTION v0.1.0"
git push origin v0.1.0
```

Для следующего выпуска используйте новый номер, например `v0.1.1`.

Если GitHub Actions сообщает об отсутствии прав, откройте `Settings` → `Actions`
→ `General` → `Workflow permissions` и включите `Read and write permissions`.

GGUF-модель не включается в Release: пользователь выбирает и скачивает её отдельно.
