use super::resource::CacheResource;

#[derive(Debug, Clone, Copy)]
pub enum InvalidationScope {
    ExpenseChange,
    RecurringChange,
    PlannedChange,
    BudgetChange,
    IncomeChange,
    ScheduleChange,
    SettingsChange,
    AccountChange,
    MoneyContextRefresh,
}

impl InvalidationScope {
    pub fn resources(self) -> &'static [CacheResource] {
        match self {
            Self::ExpenseChange => &[
                CacheResource::Expenses,
                CacheResource::Tags,
                CacheResource::Projections,
                CacheResource::ExpensePeriodView,
                CacheResource::UpcomingPayable,
            ],
            Self::RecurringChange => &[
                CacheResource::Recurring,
                CacheResource::Expenses,
                CacheResource::Tags,
                CacheResource::Projections,
                CacheResource::ExpensePeriodView,
                CacheResource::UpcomingPayable,
            ],
            Self::PlannedChange => &[
                CacheResource::Planned,
                CacheResource::Expenses,
                CacheResource::Tags,
                CacheResource::Projections,
                CacheResource::ExpensePeriodView,
                CacheResource::UpcomingPayable,
            ],
            Self::BudgetChange => &[
                CacheResource::Budgets,
                CacheResource::Expenses,
                CacheResource::Tags,
                CacheResource::Projections,
                CacheResource::ExpensePeriodView,
                CacheResource::UpcomingPayable,
            ],
            Self::IncomeChange => &[CacheResource::Income, CacheResource::Projections],
            Self::ScheduleChange => &[
                CacheResource::Schedules,
                CacheResource::Income,
                CacheResource::Projections,
                CacheResource::Settings,
                CacheResource::ExpensePeriodView,
            ],
            Self::SettingsChange => &[
                CacheResource::Settings,
                CacheResource::MoneyContext,
                CacheResource::Projections,
                CacheResource::Income,
                CacheResource::Expenses,
                CacheResource::ExpensePeriodView,
                CacheResource::UpcomingPayable,
            ],
            // Account balances are computed live (not cached); only the projection opening balance,
            // which is seeded from every account's initial amount, needs busting here.
            Self::AccountChange => &[CacheResource::Projections],
            Self::MoneyContextRefresh => &[CacheResource::MoneyContext],
        }
    }
}
