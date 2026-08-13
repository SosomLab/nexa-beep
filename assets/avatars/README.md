# 기본 아바타 원본 (12간지)

`tools/mkavatars`가 이 폴더를 읽어 `crates/nbeep-ui/assets/avatars.nbav`를 만든다.
런타임은 생성물만 보므로 **원본 크기·여백은 제각각이어도 된다** — 도구가 알파
바운딩 박스로 여백을 잘라내고 같은 비율로 정사각 캔버스에 안착시킨다.

## 파일 이름 (순서 = 12간지 · 키는 설정에 저장되므로 불변)

| 파일 | 키 | 띠 |
| --- | --- | --- |
| `01-rat.png` | `rat` | 쥐 |
| `02-ox.png` | `ox` | 소 |
| `03-tiger.png` | `tiger` | 호랑이 |
| `04-rabbit.png` | `rabbit` | 토끼 |
| `05-dragon.png` | `dragon` | 용 |
| `06-snake.png` | `snake` | 뱀 |
| `07-horse.png` | `horse` | 말 |
| `08-goat.png` | `goat` | 양 |
| `09-monkey.png` | `monkey` | 원숭이 |
| `10-rooster.png` | `rooster` | 닭 |
| `11-dog.png` | `dog` | 개 |
| `12-pig.png` | `pig` | 돼지 |

PNG(투명 배경) 권장. 이름이 달라도 도구가 순서로 잡을 수 있으나, 위 이름이면 확실하다.
