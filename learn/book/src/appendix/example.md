# 附录：完整示例

> 一个多文件命令行程序的完整示例：用 `struct` 表达数据，用 `enum` 与 `match` 分类，用 `Result` 处理校验失败，并通过模块拆分入口、模型、存储与报表。
>
> 界面不是语言层的一部分；如果程序需要 UI，应由库基于这里的普通值构建。

## 文件结构

```text
inventory/
├── main.xulo          # 命令行入口
├── model.xulo         # 数据模型与校验
├── store.xulo         # 内存集合操作
└── report.xulo        # 输出格式化
```

## `model.xulo`（数据模型与校验）

```xulo
pub enum Category {
  Tool
  Book
  Food
}

pub struct Item {
  pub id: string
  pub name: string
  pub quantity: number
  pub category: Category
}

pub fn make_item(
  id: string,
  name: string,
  quantity: number,
  category: Category
): Result<Item, string> {
  if id == "" {
    Result::Err("id must not be empty")
  } else if quantity < 0 {
    Result::Err("quantity must be non-negative")
  } else {
    Result::Ok(Item(id: id, name: name, quantity: quantity, category: category))
  }
}

pub fn category_label(category: Category): string {
  match category {
    Category::Tool => "tool"
    Category::Book => "book"
    Category::Food => "food"
  }
}

pub fn describe(item: Item): string {
  item.name + " (" + item.id + ") x" + str(item.quantity) + " [" + category_label(item.category) + "]"
}
```

## `store.xulo`（集合操作）

```xulo
import type { Item } from "./model"

pub struct Inventory {
  pub items: List<Item>
}

pub fn empty(): Inventory {
  Inventory(items: [])
}

pub fn add(inventory: Inventory, item: Item): Inventory {
  Inventory(items: inventory.items + [item])
}

pub fn find(inventory: Inventory, id: string): Item? {
  for item in inventory.items {
    if item.id == id {
      return item
    }
  }
  null
}

pub fn total_quantity(inventory: Inventory): number {
  let mut total = 0
  for item in inventory.items {
    total = total + item.quantity
  }
  total
}
```

## `report.xulo`（格式化输出）

```xulo
import type { Item } from "./model"
import { describe } from "./model"
import type { Inventory } from "./store"
import { total_quantity } from "./store"

pub fn header(): string {
  "== Inventory =="
}

pub fn line(item: Item): string {
  "- " + describe(item)
}

pub fn summary(inventory: Inventory): string {
  "total quantity: " + str(total_quantity(inventory))
}
```

## `main.xulo`（入口）

```xulo
import { make_item, describe, Category } from "./model"
import type { Inventory } from "./store"
import { empty, add, find } from "./store"
import { header, line, summary } from "./report"

fn add_one(
  inventory: Inventory,
  id: string,
  name: string,
  quantity: number,
  category: Category
): Inventory {
  match make_item(id, name, quantity, category) {
    Result::Ok(item) => add(inventory, item)
    Result::Err(reason) => {
      print("skip " + id + ": " + reason)
      inventory
    }
  }
}

fn main() {
  let mut inventory = empty()
  inventory = add_one(inventory, "p-1", "Hammer", 3, Category::Tool)
  inventory = add_one(inventory, "p-2", "Notebook", 12, Category::Book)
  inventory = add_one(inventory, "p-3", "Coffee", -1, Category::Food)   // 被拒绝

  print(header())
  for item in inventory.items {
    print(line(item))
  }
  print(summary(inventory))

  let found = find(inventory, "p-2")
  if found != null {
    print("lookup: " + describe(found))
  }
}
```

## 运行

```bash
xulo run main.xulo
```

```text
skip p-3: quantity must be non-negative
== Inventory ==
- Hammer (p-1) x3 [tool]
- Notebook (p-2) x12 [book]
total quantity: 15
lookup: Notebook (p-2) x12 [book]
```
