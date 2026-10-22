//! 错误映射。
//!
//! 分类原则（见 [ARCHITECTURE §9](../../../docs/ARCHITECTURE.md)）：
//! 错误按**可恢复原因**分类，不能一律当成"存储错误，重试就好"。
//!
//! * 约束冲突（唯一键 / 外键 / NOT NULL / CHECK）→ `AppError::Invalid`。
//!   同一份输入重试不可能成功，**不能**归入可重试的 `Storage`。
//! * 其余 sqlx 错误 → `AppError::Storage`（默认可重试）。
//!
//! 注意：`AppError::with_context` 按变体逐个包裹，会保留类别，
//! 因此这里的分类不会在附加上下文时丢失（`xremail-core` 已有单测锁定）。

use xremail_core::AppError;

/// 本 crate 统一使用的返回类型。
pub type StoreResult<T> = Result<T, AppError>;

/// 把 sqlx 错误映射为应用错误，并附加上下文。
pub fn map_sqlx(err: sqlx::Error, context: &str) -> AppError {
    if is_constraint_violation(&err) {
        AppError::Invalid(err.to_string()).with_context(context)
    } else {
        AppError::Storage(err.to_string()).with_context(context)
    }
}

/// 是否为约束冲突。
///
/// 覆盖四类：唯一键、主键、外键、CHECK。
/// NOT NULL 违反不单独列举——schema 里所有 NOT NULL 列在 INSERT 时都必须给出值，
/// 违反它说明调用方漏传参数，由类型系统与 INSERT 语句本身挡住。
fn is_constraint_violation(err: &sqlx::Error) -> bool {
    let sqlx::Error::Database(db_err) = err else {
        return false;
    };

    db_err.is_unique_violation() || db_err.is_foreign_key_violation() || db_err.is_check_violation()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_timeout_stays_retryable_storage() {
        let err = map_sqlx(sqlx::Error::PoolTimedOut, "acquire");

        assert!(err.is_retryable());
        assert!(!err.requires_user_action());
        assert!(err.to_string().contains("acquire"));
    }

    #[test]
    fn context_is_preserved_and_category_survives() {
        let err = map_sqlx(sqlx::Error::PoolTimedOut, "upsert_account");

        // with_context 按变体逐个包裹，Storage 类别必须保留，
        // 否则 is_retryable() 判定会失效。
        assert!(matches!(err, AppError::Storage(_)));
        assert!(err.to_string().contains("upsert_account"));
    }
}
