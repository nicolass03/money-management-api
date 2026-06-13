#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CacheResource {
    Settings,
    Expenses,
    Recurring,
    Planned,
    Budgets,
    Income,
    Schedules,
    Tags,
    Projections,
    MoneyContext,
    ExpensePeriodView,
    UpcomingPayable,
}
