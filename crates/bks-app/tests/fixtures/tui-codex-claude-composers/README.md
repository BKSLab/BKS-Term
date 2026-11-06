# Codex / Claude: общая проекция поля ввода

`conpty-hidden-menu.vt` — 4 071 байт настоящего ConPTY output от изолированного
renderer-equivalent composer fixture, 100 × 30. SHA256:
`AE419155DE3273F32B0DE303F67E52E73DAC6CA312F78A7F5BC32453C3E0B77A`.
Это транспортная регрессия, не авторизованная сессия Codex. До байта 3 707
видны пустое поле и каретка (3,2), затем принят `/`. До байта 3 928 ConPTY
выдаёт завершённый sync frame без каретки и переносит поле под меню;
оставшиеся байты возвращают каретку (6,3) и точный `/`. Session regression
проверяет возврат нижнего поля без публикации private baseline, а также
clear/expiry/control/resize. Временные данные не содержат пользовательских
настроек, авторизации или запросов к модели.

`conpty-hidden-navigation.vt` — 4 495 байт ConPTY output того же отдельного
fixture, 100 × 30, после двух явных writes `/` (2F) и Down (1B5B42).
SHA256 `836874014B3B0758A80F4A06EA616B78E2D09D73EA1D11E68CB8A8FFA6E45E2F`.
Endpoints: пустое поле 3767, hidden slash frame 3988, видимое меню 4131,
hidden Down frame 4352, видимое меню с выбранным Bravo 4495. Hidden frame
оставляет строку поля пустой; tail возвращает тот же `/` и каретку (6,3).
Стрелка отменяет доказательство вставки. Отдельная continuity-проверка
восстанавливает только неизменное исходное поле с точным оформлением;
expiry, resize и смена режима запрещают восстановление.

`conpty-hidden-home-right.vt` — 4 857 байт, SHA256
`76E435A48EC3B4585C19BD0ACF948DDE31EC67F718BA94EE474B90738A35ECCB`.
Тот же isolated ConPTY 100 × 30; явные writes: `Ж界🙂`, Home, Right.
Пустое поле заканчивается на 3767, Unicode на 4265, hidden Home на 4486,
Home tail на 4561, hidden Right на 4782, Right tail на 4857.
Home не должен превращать подтверждённый текст в unentered hint только
из-за каретки в начале. Регрессия проверяет неизменный текст и UTF-16 caret
4 → 0 → 1, включая скрытые промежуточные кадры.

`codex-0.164.0-auth-startup.vt` и `claude-2.1.240-offline-startup.vt` — реальные
запуски установленных бинарников в отдельном ConPTY **110 × 36**. HOME,
USERPROFILE, APPDATA и каталоги настроек изолированы; унаследованные переменные
удалены, HTTP/HTTPS/ALL_PROXY направлены на закрытый loopback port 9.
Пользовательский ввод, вход в аккаунт, подтверждение доверия и запросы к LLM
не выполнялись. Обе собственные PTY закрыты после захвата.

- Codex 0.164.0 остановился на выборе способа входа. SHA256 бинарника:
  `6F267830B029EE25D9339864AA867CD45DE9F8FD4C5EA007D2E63C13A0C66D36`.
- Claude Code 2.1.240 завершился с `ConnectionRefused` при проверке доступности
  сервиса. SHA256 бинарника:
  `F7EE87C58D315BEDBD38FC8923CE7B955DBBAB2AFE50D9D15C059463A70869DA`.

Исходные composer-сценарии в `tui_projection.rs` **source-equivalent**.
Ниже добавлены реальные offline composer-записи обоих CLI; авторизованная
сессия и модель для них не использовались. Геометрия Codex проверена по
[chat_composer.rs](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/tui/src/bottom_pane/chat_composer.rs)
и [composer_layout.rs](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/tui/src/bottom_pane/chat_composer/composer_layout.rs):
цветная область, пустые верхняя/нижняя строки, `›`, две колонки prefix,
правая колонка padding, отдельный footer. Без ответа palette probe фон может
совпадать с остальным терминалом; отдельная проверка использует бесцветный вариант.

Геометрия Claude проверена по встроенному renderer установленного бинарника
2.1.240: `bMo`/`lPs` (offset 322822927 / 322823364) рисуют pointer + NBSP,
либо `$` + NBSP при screen-reader режиме; блок около offset 322871567 имеет
горизонтальные границы без боковых, отдельно расположенный multiline textarea
и программный inverse cursor. Fixtures проверяют Unicode, переносы, стрелки,
Home, сохранение пользовательских отступов после двух колонок prefix.

Проектор не распознаёт названия приложений и не отправляет ввод. Начальный
текст padded-поля появляется в нижнем нативном поле только после точного
совпадения с новым VT-кадром. Полный bracketed paste допускает CR/LF с
нормализацией к CRLF; raw Enter, ESC, navigation и неполные paste исключены
из подтверждения. Исходный VT и документ остаются неизменными.

## Реальные offline composers и редактирование, 02.11.2026

Оба CLI запущены в собственном ConPTY 100 × 29, с отдельными HOME, USERPROFILE,
APPDATA и каталогами настроек. Cwd — новый пустой каталог вне Git под
`Temp/BKSComposerFixture/`. Proxy направлен на закрытый loopback port 9;
унаследованные переменные удалены. Ни Enter, ни запрос к модели не отправлялись.
На всех десяти endpoints процессы были живы, exit code отсутствовал; после
захвата завершены только собственные ConPTY. Helper и JSON фаз находятся в
build directory `target/codex-input/`.

- Codex 0.164.0: `--no-alt-screen --ask-for-approval never --sandbox read-only`.
  Отдельный provider `bks-offline` имеет Responses URL `http://127.0.0.1:9/v1`,
  `requires_openai_auth=false`; trusted только пустой тестовый cwd. CODEX_HOME
  для editor-записи — короткий `Temp/BKSCx/codex-live2`, чтобы daemon socket
  не превышал системное ограничение пути. Устаревшие пробы с длинным socket
  path не используются как доказательство живого редактирования.
- Claude Code 2.1.240: `--bare`, `ANTHROPIC_BASE_URL=http://127.0.0.1:9` и
  фиксированный fake API key. Только отдельный fixture config задаёт тему,
  завершённый onboarding, trust пустого cwd и разрешение этого fake key.
  Пользовательские auth/config не читались и не менялись. Исходный пустой
  ruled composer использует программный inverse cursor.

`codex-0.164.0-offline-composer.vt` — отдельный первоначальный startup,
4 164 байта, SHA256
`807416B0D50E0074DF186276312B8FA225242C860A89C158C4A10BE8789BFFCD`.
Он воспроизводит `Ask Codex to do anything` с кареткой перед hint. Пустой
padding непосредственно соседствует с footer; прежний parser требовал ещё
одну пустую строку и попадал в bare fallback с буквальным hint.

`codex-0.164.0-offline-edit.vt` — 6 101 байт, SHA256
`6840DA70787742BB1C102495D2A1565588968FA46C0F85A9FC6604DFCC70131F`.
`claude-2.1.240-offline-edit.vt` — 3 791 байт, SHA256
`5F8FE05221F8CB8C59763ABD3B7180F67805DDB982207EE088C0DD3F782843AD`.

| Фаза / явный editor input | Codex exclusive byte end | Claude exclusive byte end |
| --- | ---: | ---: |
| Startup | 4 132 | 2 676 |
| Bracketed paste `Ж界🙂` | 4 188 | 2 750 |
| Home | 4 222 | 2 780 |
| Right | 4 242 | 2 801 |
| Backspace | 4 287 | 2 832 |
| End | 4 307 | 2 872 |
| Clear: два Backspace / Ctrl+U | 4 375 | 3 017 |
| `/` | 5 267 | 3 469 |
| Down | 5 540 | 3 690 |
| Escape | 6 101 | 3 791 |

Replay проверяет пустой native draft для hint, точное source acknowledgement
вставки, UTF-16 caret 4 → 0 → 1, удаление одного исходного символа, очистку
и возврат hint, slash-меню, Down/Esc и отсутствие второго экземпляра draft
в верхнем документе. Отдельная source-equivalent регрессия проверяет footer,
выровненный вправо, как на пользовательском снимке с weekly limit; parser
не использует текст hint, название CLI или конкретную фразу footer.

Реальное авторизованное редактирование Codex/Claude и NVDA требуют отдельной
ручной приёмки; эти fixtures её не заменяют.
