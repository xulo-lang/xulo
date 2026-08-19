# 附录：完整示例

> 一个多文件 UI 应用的完整示例，覆盖枚举、类型别名、Store、组件、Props、事件、双向绑定与异步。
>
> 依赖 `@xulo/ui` / `@xulo/store` 等外部运行时，以及目标平台的全局 API（如 `fetch`）。本示例展示语言的目标形态；当前编译器对外部 API 的调用在纯静态检查（`xulo check`）下可能提示「未声明 / 不可调用」，属预期行为。

## 文件结构

```text
project/
├── main.xulo          # 应用入口
├── stores/
│   └── app.xulo       # 全局状态
├── components/
│   └── button.xulo    # 自定义组件
└── pages/
    └── profile.xulo   # 页面组件
```

## `types.xulo`（共享类型）

```xulo
// 枚举
pub enum Theme {
  Light
  Dark
  System
}

pub enum Status {
  Active
  Inactive
  Pending
}

// 类型别名
pub type User = {
  id: string
  name: string
  email: string
  status: Status
}

pub type Result<T> = {
  data: T?
  error: string?
}
```

## `stores/app.xulo`

```xulo
import { createStore } from "@xulo/store"
import type { User, Theme } from "../types"

type AppState = {
  user: User?
  theme: Theme
  notifications: list<string>
  loading: boolean
  error: string?
}

// ✅ 对象字面量用括号包裹
fn setUser(state: AppState, user: User?): AppState {
  ({ ...state, user: user, loading: false })
}

fn setTheme(state: AppState, theme: Theme): AppState {
  ({ ...state, theme: theme })
}

fn setLoading(state: AppState, loading: boolean): AppState {
  ({ ...state, loading: loading })
}

fn setError(state: AppState, error: string): AppState {
  ({ ...state, error: error, loading: false })
}

fn addNotification(state: AppState, message: string): AppState {
  ({ ...state, notifications: state.notifications + [message] })
}

// 注意：当前实现暂不支持显式泛型调用（`createStore<AppState>`），故省略类型实参。
pub const useAppStore = createStore(
  {
    user: null,
    theme: Theme::Light,
    notifications: [],
    loading: false,
    error: null
  },
  {
    setUser: setUser,
    setTheme: setTheme,
    setLoading: setLoading,
    setError: setError,
    addNotification: addNotification
  }
)

// ✅ 异步函数直接调用 useAppStore()，不使用 @Store
pub fn fetchUser(id: string): async {
  const store = useAppStore()

  store.actions.setLoading(true)
  try {
    let response = await fetch("/api/users/" + id)
    let data = await response.json()
    store.actions.setUser(data)
    store.actions.addNotification("User loaded: " + data.name)
  } catch (e) {
    store.actions.setError(e.message)
  }
}
```

## `components/button.xulo`

```xulo
import type { Theme } from "../types"

pub enum ButtonVariant {
  Primary
  Secondary
  Outline
  Ghost
}

pub fn PrimaryButton(
  label: string,
  onClick: fn()? = null,
  disabled: boolean? = false
): View {
  Button(
    variant: ButtonVariant::Primary,
    onClick: onClick,
    disabled: disabled,
    width: "100%"
  ) {
    Text(label, weight: "bold")
  }
}

pub fn OutlineButton(
  label: string,
  onClick: fn()? = null
): View {
  Button(variant: ButtonVariant::Outline, onClick: onClick) {
    Text(label)
  }
}
```

## `pages/profile.xulo`

```xulo
import { useAppStore, fetchUser } from "../stores/app"
import { PrimaryButton, OutlineButton } from "../components/button"
import type { User, Theme, Status } from "../types"

type Props = {
  id: string
}

pub fn UserProfile(props: Props): View {
  // ✅ @State/@Store/@Effect 只能在返回 View 的函数顶层使用
  @State let editing: boolean = false
  @State let editName: string = ""
  @Store const { user, theme, loading, error } = useAppStore()
  @Store const { setTheme, addNotification } = useAppStore()

  @Effect fn() {
    fetchUser(props.id)
  }

  // 当 user 变化时更新编辑字段
  @Effect fn() {
    if user != null {
      editName = user.name
    }
  }, [user]

  Card(radius: "lg", shadow: "sm") {
    VStack(spacing: 16) {
      HStack {
        Text("User Profile", weight: "bold", size: 24)
        Spacer()
        Button(onClick: fn() {
          let newTheme = match theme {
            Theme::Light => Theme::Dark
            Theme::Dark => Theme::Light
            Theme::System => Theme::Light
          }
          setTheme(newTheme)
        }) {
          Text(match theme {
            Theme::Light => "🌙"
            Theme::Dark => "☀️"
            Theme::System => "💻"
          })
        }
      }

      if loading {
        Text("Loading...", color: "$theme.text.secondary")
      } else if error != null {
        Text("Error: " + error, color: "$theme.danger")
      } else if user != null {
        VStack(spacing: 8) {
          if editing {
            Input(value: $editName)
            HStack(spacing: 12) {
              PrimaryButton(label: "Save", onClick: fn() {
                // 保存逻辑
                editing = false
                addNotification("Name updated to: " + editName)
              })
              OutlineButton(label: "Cancel", onClick: fn() {
                editing = false
                editName = user?.name ?? ""
              })
            }
          } else {
            Text(user.name, weight: "bold", size: 18)
            Text(user.email, color: "$theme.text.secondary")
            Text("Status: " + match user.status {
              Status::Active => "🟢 Active"
              Status::Inactive => "🔴 Inactive"
              Status::Pending => "🟡 Pending"
            })
            PrimaryButton(label: "Edit Profile", onClick: fn() {
              editing = true
            })
          }
        }
      } else {
        Text("User not found", color: "$theme.warning")
      }
    }
  }
}
```

## `main.xulo`

```xulo
import { Screen, VStack, Center } from "@xulo/ui"
import { UserProfile } from "./pages/profile"
import { useAppStore } from "./stores/app"
import type { Theme } from "./types"

fn main(): View {
  @Store const { theme } = useAppStore()

  Screen(
    background: match theme {
      Theme::Light => "#ffffff"
      Theme::Dark => "#1a1a2e"
      Theme::System => "#f5f5f5"
    }
  ) {
    Center {
      VStack(spacing: 24) {
        Text("Xulo App", weight: "bold", size: 28)
        UserProfile(id: "123")
      }
    }
  }
}
```
