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
    MoneyContextRefresh,
}

impl InvalidationScope {
    pub fn resources(self) -> &'static [CacheResource] {
        match self {
            Self::ExpenseChange => &[CacheResource::Expenses, CacheResource::Tags, CacheResource::Projections],
            Self::RecurringChange => &[
                CacheResource::Recurring,
                CacheResource::Expenses,
                CacheResource::Tags,
                CacheResource::Projections,
            ],
            Self::PlannedChange => &[
                CacheResource::Planned,
                CacheResource::Expenses,
                CacheResource::Tags,
                CacheResource::Projections,
            ],
            Self::BudgetChange => &[
                CacheResource::Budgets,
                CacheResource::Expenses,
                CacheResource::Tags,
                CacheResource::Projections,
            ],
            Self::IncomeChange => &[CacheResource::Income, CacheResource::Projections],
            Self::ScheduleChange => &[
                CacheResource::Schedules,
                CacheResource::Income,
                CacheResource::Projections,
                CacheResource::Settings,
            ],
            Self::SettingsChange => &[
                CacheResource::Settings,
                CacheResource::MoneyContext,
                CacheResource::Projections,
                CacheResource::Income,
                CacheResource::Expenses,
            ],
            Self::MoneyContextRefresh => &[CacheResource::MoneyContext],
        }
    }
}
