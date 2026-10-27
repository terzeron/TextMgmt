# Справочник по API

Всё, что описано на этой странице, реэкспортируется из `pyferrite::prelude`.

---

## Функции верхнего уровня

### `read(path) -> Result<Value>`

Читает файл с настройками по умолчанию.

| Параметр | Тип | Назначение |
|---|---|---|
| `path` | `impl AsRef<Path>` | Читаемый файл. Формат определяется по ведущим байтам-сигнатуре; если она неоднозначна, решает расширение. |

Возвращает [`Value`](types.md#value): `Array` для `.npy`, `Dict` для
контейнерных форматов и то, что содержит граф объектов, для pickle.

```rust,no_run
# use pyferrite::prelude::*;
let v = read("weights.npz")?;
println!("{}", v.type_name());
# Ok::<(), pyferrite::Error>(())
```

### `read_with(path, opts) -> Result<Value>`

То же самое, но с управлением приведением типов, лимитом памяти и определением
формата.

| Параметр | Тип | Назначение |
|---|---|---|
| `path` | `impl AsRef<Path>` | Читаемый файл |
| `opts` | `&ReadOptions` | См. [ReadOptions](#readoptions) |

```rust,no_run
# use pyferrite::prelude::*;
// Привести все вещественные к f32 и ограничить выделение памяти 512 МиБ.
let opts = ReadOptions::new().float_as(FloatKind::F32).max_alloc(512 << 20);
let v = read_with("big.pkl", &opts)?;
# Ok::<(), pyferrite::Error>(())
```

### `write(path, value) -> Result<()>`

Записывает `value` с настройками по умолчанию. Формат берётся из расширения,
поэтому у пути должно быть расширение, известное библиотеке.

| Параметр | Тип | Назначение |
|---|---|---|
| `path` | `impl AsRef<Path>` | Куда писать |
| `value` | `&Value` | Что писать. `.npy` принимает один массив, остальные форматы — произвольную вложенность. |

### `write_with(path, value, opts) -> Result<()>`

| Параметр | Тип | Назначение |
|---|---|---|
| `path` | `impl AsRef<Path>` | Куда писать |
| `value` | `&Value` | Что писать |
| `opts` | `&WriteOptions` | См. [WriteOptions](#writeoptions) |

Проверки выполняются до создания файла, поэтому отклонённая запись не оставляет
после себя обрубка.

### `read_lazy(path, opts) -> Result<LazyReader>`

Открывает файл для чтения порциями. Поддерживается для `.npy`, `.npz` и `.h5`.
См. [lazy.md](lazy.md).

### `write_lazy(path, dtype, shape, opts) -> Result<LazyWriter>`

| Параметр | Тип | Назначение |
|---|---|---|
| `path` | `impl AsRef<Path>` | Куда писать |
| `dtype` | `DType` | Тип хранимых элементов. Порции любого типа приводятся к нему. |
| `shape` | `&[usize]` | Полная итоговая форма. Объявляется заранее, потому что заголовок предшествует данным. |
| `opts` | `&WriteOptions` | Сжатие, политика приведения, принудительный формат |

### `convert(src, dst, read_opts, write_opts) -> Result<()>`

Читает один файл и записывает его в другом формате.

```rust,no_run
# use pyferrite::prelude::*;
convert("model.pt", "model.h5", &ReadOptions::new(), &WriteOptions::new())?;
# Ok::<(), pyferrite::Error>(())
```

### `flatten_arrays(&Value) -> Vec<(String, &Array)>`

Собирает все массивы значения, ключом служит путь через точку. Вложенные
словари соединяют ключи точкой, поэтому torch-овский `state_dict` выглядит
ровно так, как его печатает Python: `layer1.weight`, `layer1.bias` и так далее.
Элементы списков и кортежей дают свой индекс.

---

## `ReadOptions`

Каждый метод принимает `self` и возвращает `Self`, поэтому вызовы можно
соединять в цепочку. Все параметры необязательны.

| Метод | Тип | По умолчанию | Действие |
|---|---|---|---|
| `lazy(bool)` | `bool` | `false` | Отмечает намерение читать потоком. Настоящая точка входа — `read_lazy`; этот флаг нужен, чтобы хранить выбор в конфигурации. |
| `chunk_elements(usize)` | `usize` | `1 << 20` | Элементов в одной порции. Расход памяти — примерно это число, умноженное на размер элемента. |
| `int_as(IntKind)` | `IntKind` | нет | Привести все целочисленные массивы к этой разрядности: `I8` … `I64`, `U8` … `U64`. |
| `float_as(FloatKind)` | `FloatKind` | нет | Привести все вещественные массивы: `F8E4M3`, `F8E5M2`, `F16`, `BF16`, `F32`, `F64`, `F128`. |
| `cast_policy(CastPolicy)` | `CastPolicy` | `Strict` | Что делать, если значение не помещается. См. [types.md](types.md#приведение-типов). |
| `format(Format)` | `Format` | авто | Не определять формат, а взять указанный. |
| `allow_unknown_globals(bool)` | `bool` | `false` | Инертно сохранять вызовы из чёрного списка (`os`, `eval` и т. п.) вместо отказа читать файл. Незнакомые классы сохраняются инертно в любом режиме, и ничего не исполняется. |
| `max_alloc(usize)` | `usize` | 8 ГиБ | Отклонять любое единичное выделение памяти сверх этого. Защита от враждебного или испорченного заголовка. |
| `keep_record_arrays(bool)` | `bool` | `false` | Оставлять структурированные массивы numpy как есть, а не превращать их в `Frame`. |

```rust,no_run
# use pyferrite::prelude::*;
let opts = ReadOptions::new()
    .float_as(FloatKind::F16)
    .cast_policy(CastPolicy::Saturate)   // обрезать по краям, а не падать
    .max_alloc(1 << 30)
    .chunk_elements(4096);
# let _ = opts;
```

---

## `WriteOptions`

| Метод | Тип | По умолчанию | Действие |
|---|---|---|---|
| `lazy(bool)` | `bool` | `false` | Отметка намерения, зеркальная `ReadOptions::lazy`. |
| `chunk_elements(usize)` | `usize` | `1 << 20` | Размер порции при потоковой записи. |
| `int_as(IntKind)` | `IntKind` | нет | Хранить целые в этой разрядности. |
| `float_as(FloatKind)` | `FloatKind` | нет | Хранить вещественные в этой разрядности — именно так данные f64 записываются как f16. |
| `dtype(DType)` | `DType` | нет | Задать один конкретный тип элемента, перекрывая `int_as` и `float_as`. |
| `shape(impl Into<Vec<usize>>)` | `Vec<usize>` | форма источника | Форма, которую должен объявить файл. Число элементов обязано совпасть, иначе `Error::Shape`. Именно так различаются `2`, `[2]` и `[[2]]`. |
| `cast_policy(CastPolicy)` | `CastPolicy` | `Strict` | Поведение при сужении. |
| `format(Format)` | `Format` | из расширения | Взять указанный писатель независимо от имени файла. |
| `compression(Compression)` | `Compression` | `None` | `None`, `Compression::deflate()`, `Compression::zlib()`. Действует на элементы `.npz`, на `.joblib` и `.pt`. |
| `pickle_protocol(u8)` | `u8` | `4` | Протокол для `.pkl`, `.pt` и `.joblib`. Поддерживаются 2–5; протокол 2 самый переносимый. |
| `fortran_order(bool)` | `bool` | `false` | Писать `.npy` по столбцам. |

```rust,no_run
# use pyferrite::prelude::*;
# let value = Value::Array(pyferrite::value::Array::F64(ndarray::ArrayD::zeros(ndarray::IxDyn(&[6]))));
let opts = WriteOptions::new()
    .float_as(FloatKind::F32)
    .shape([2, 3])
    .compression(Compression::deflate())
    .pickle_protocol(2);
write_with("out.npz", &value, &opts)?;
# Ok::<(), pyferrite::Error>(())
```

---

## `Value`

Отражение графа объектов Python на стороне Rust.

| Вариант | Соответствие в Python |
|---|---|
| `None` | `None` |
| `Bool(bool)` | `bool` |
| `Int(i64)` | `int`, который помещается |
| `BigInt(Vec<u8>)` | `int` произвольной точности, дополнительный код, младшие байты первыми |
| `Float(f64)` | `float` |
| `Complex(Complex64)` | `complex` |
| `Str(String)` | `str` |
| `Bytes(Vec<u8>)` | `bytes`, `bytearray` |
| `List(Vec<Value>)` | `list` |
| `Tuple(Vec<Value>)` | `tuple` |
| `Set(Vec<Value>)` | `set`, `frozenset` |
| `Dict(Dict)` | `dict`, `OrderedDict` — порядок вставки сохраняется |
| `Array(Array)` | `np.ndarray`, `torch.Tensor` |
| `Frame(Frame)` | `pandas.DataFrame`, `polars.DataFrame`, структурированный массив numpy |
| `Object(PyObject)` | любой другой класс, захваченный инертно |

### Аксессоры

Только для чтения: `as_str`, `as_f64`, `as_i64`, `as_array`, `as_dict`,
`as_frame`, `as_object`, `type_name`.

Изменяющие: `as_array_mut`, `as_dict_mut`, `as_frame_mut` и `arrays_mut()`,
который выдаёт `&mut Array` для каждого массива в любом месте дерева.

```rust,no_run
# use pyferrite::prelude::*;
let mut v = read("model.pkl")?;
if let Some(d) = v.as_dict_mut() {
    d.insert(Value::Str("epoch".into()), Value::Int(12));
}
for a in v.arrays_mut() {
    if let Some(f) = a.as_f64_mut() {
        let mean = f.iter().sum::<f64>() / f.len() as f64;
        f.iter_mut().for_each(|x| *x -= mean);   // центрируем на месте
    }
}
write("model.pkl", &v)?;
# Ok::<(), pyferrite::Error>(())
```

---

## `Dict`

Словарь с сохранением порядка вставки — потому что словари Python упорядочены,
и обход туда-обратно не должен перемешивать `state_dict`.

`insert`, `get`, `get_mut`, `get_by`, `remove`, `contains_key`, `iter`,
`iter_mut`, `keys`, `values_mut`, `into_entries`. `get` и его соседи принимают
`&str` напрямую — для обычного случая строковых ключей.

## `Array`

Перечисление над `ndarray::ArrayD<T>` для всех двадцати типов элементов.
`dtype()`, `shape()`, `len()`, `ndim()`, `reshape(&[usize])`,
`cast(&DType, CastPolicy)`, а также аксессоры вида `as_f32()` / `as_f32_mut()` /
`into_f32()` для каждого типа.

## `Frame` и `Series`

`Frame` — список именованных `Series`, каждая из которых содержит `Array` и
необязательную битовую маску валидности для пропусков. `from_columns`,
`height`, `width`, `names`, `column`, `column_mut`, `push_column`,
`drop_column`.

## `PyObject`

Во что превращается неизвестный класс Python: `module`, `name`, `args`,
`kwargs`, `state`, `list_items`, `dict_items`, а также `qualname()`, `attr()` и
`attr_mut()`. Ничего не вызывается; аргументы конструктора просто записываются,
так что их можно изучить, переписать и записать объект обратно.
