//! Session-local Forms state and PostgreSQL transaction boundaries.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};
use tokio_postgres::{Client, NoTls};

type Fields = BTreeMap<String, String>;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

const HEADER: &[(&str, &str)] = &[
    ("order-id", "id"),
    ("customer-id", "customer_id"),
    ("date-ordered", "date_ordered"),
    ("date-shipped", "date_shipped"),
    ("rep-id", "sales_rep_id"),
    ("payment-type", "payment_type"),
    ("order-filled", "order_filled"),
];
const LINE: &[(&str, &str)] = &[
    ("item-id", "item_id"),
    ("product-id", "product_id"),
    ("price", "price"),
    ("quantity", "quantity"),
    ("shipped", "quantity_shipped"),
];

const QUERY_KEY_MESSAGE: &str =
    "FRM-41009: Function key not allowed.  Press Ctrl+K for list of valid keys.";

/// One user gesture from the embedded UI; typing does not imply validation.
#[derive(Clone, Deserialize)]
pub struct Command {
    action: String,
    #[serde(default)]
    item: String,
    #[serde(default)]
    value: String,
    line: Option<usize>,
}

#[derive(Clone, Serialize)]
struct Focus {
    item: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    line: Option<usize>,
}

impl Focus {
    fn header(item: &str) -> Self {
        Self {
            item: item.into(),
            line: None,
        }
    }
    fn line(item: &str, line: usize) -> Self {
        Self {
            item: item.into(),
            line: Some(line),
        }
    }
}

#[derive(Clone, Default)]
struct Item {
    fields: Fields,
    original: Option<Value>,
    validated_product: String,
    total: Option<i64>,
    total_exact: Option<(i128, i32)>,
    dirty: bool,
}

#[derive(Clone)]
struct Order {
    fields: Fields,
    original: Option<Value>,
    items: Vec<Item>,
    removed: Vec<Value>,
    details_loaded: bool,
    validated_customer: String,
    dirty: bool,
}

impl Order {
    fn blank(today: &str) -> Self {
        Self {
            fields: Fields::from([
                ("date-ordered".into(), today.into()),
                ("payment-type".into(), "CASH".into()),
                ("order-filled".into(), "N".into()),
            ]),
            original: None,
            items: vec![Item::default()],
            removed: vec![],
            details_loaded: true,
            validated_customer: String::new(),
            dirty: false,
        }
    }

    fn changed(&self) -> bool {
        self.dirty || self.items.iter().any(|item| item.dirty) || !self.removed.is_empty()
    }

    fn total(&self) -> Option<i64> {
        let totals: Vec<(i128, i32)> = self
            .items
            .iter()
            .filter_map(|item| item.total_exact)
            .collect();
        let scale = totals.iter().map(|(_, scale)| *scale).max()?;
        let mut sum = 0_i128;
        for (amount, item_scale) in totals {
            let shift = u32::try_from(scale.checked_sub(item_scale)?).ok()?;
            sum = sum.checked_add(amount.checked_mul(10_i128.checked_pow(shift)?)?)?;
        }
        round_cents(sum, scale)
    }

    fn document(&self) -> Value {
        let mut doc = document(&self.fields, HEADER);
        doc["total"] = self
            .original
            .as_ref()
            .and_then(|orig| orig.get("total"))
            .cloned()
            .unwrap_or(Value::Null);
        doc
    }
}

#[derive(Clone, Serialize)]
struct Alert {
    title: String,
    text: String,
    buttons: Vec<String>,
    #[serde(skip)]
    continuation: String,
    #[serde(skip)]
    line: Option<usize>,
}

/// A form session owns its edit buffer; no database changes are made before save.
pub struct Session {
    client: Client,
    today: String,
    orders: Vec<Order>,
    current: usize,
    deleted: Vec<Value>,
    query: bool,
    criteria: Fields,
    focus: Option<Focus>,
    immediate: bool,
    auto_query: bool,
    sort: String,
    alert: Option<Alert>,
    messages: Vec<String>,
    closed: bool,
    products: Vec<Value>,
    transaction_open: bool,
    locks: HashSet<String>,
    pending_edit: Option<Command>,
}

fn text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(value) => value.clone(),
        _ => value.to_string(),
    }
}

fn fields(doc: &Value, mapping: &[(&str, &str)]) -> Fields {
    mapping
        .iter()
        .map(|(item, column)| ((*item).into(), text(&doc[column])))
        .collect()
}

fn document(fields: &Fields, mapping: &[(&str, &str)]) -> Value {
    let mut doc = json!({});
    for (item, column) in mapping {
        let value = fields.get(*item).cloned().unwrap_or_default();
        doc[*column] = if value.is_empty() {
            Value::Null
        } else {
            json!(value)
        };
    }
    doc
}

fn get(fields: &Fields, item: &str) -> String {
    fields.get(item).cloned().unwrap_or_default()
}

fn decimal_parts(value: &str) -> Option<(i128, i32)> {
    // Decimal arithmetic keeps monetary summaries independent of binary float rounding.
    let value = value.trim();
    let (base, exponent) = if let Some((base, exponent)) = value.split_once(['e', 'E']) {
        (base, exponent.parse::<i32>().ok()?)
    } else {
        (value, 0)
    };
    let negative = base.starts_with('-');
    let unsigned = base.strip_prefix(['-', '+']).unwrap_or(base);
    let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    let digits = format!("{whole}{fraction}");
    if digits.is_empty() || !digits.bytes().all(|digit| digit.is_ascii_digit()) {
        return None;
    }
    let amount = digits.parse::<i128>().ok()?;
    let scale = i32::try_from(fraction.len()).ok()?.checked_sub(exponent)?;
    Some((if negative { -amount } else { amount }, scale))
}

fn round_cents(amount: i128, scale: i32) -> Option<i64> {
    let shift = 2_i32.checked_sub(scale)?;
    let rounded = if shift >= 0 {
        amount.checked_mul(10_i128.checked_pow(shift as u32)?)?
    } else if let Some(divisor) = 10_i128.checked_pow(shift.unsigned_abs()) {
        let quotient = amount / divisor;
        if (amount % divisor).abs() >= (divisor + 1) / 2 {
            quotient.checked_add(amount.signum())?
        } else {
            quotient
        }
    } else {
        0
    };
    rounded.try_into().ok()
}

fn cents(value: &str) -> Option<i64> {
    let (amount, scale) = decimal_parts(value)?;
    round_cents(amount, scale)
}

fn money(amount: i64) -> String {
    let sign = if amount < 0 { "-" } else { "" };
    format!(
        "{sign}{}.{:02}",
        amount.unsigned_abs() / 100,
        amount.unsigned_abs() % 100
    )
}

fn calculate(item: &mut Item) {
    item.total_exact = decimal_parts(&get(&item.fields, "price"))
        .zip(decimal_parts(&get(&item.fields, "shipped")))
        .and_then(|((price, price_scale), (shipped, shipped_scale))| {
            Some((
                price.checked_mul(shipped)?,
                price_scale.checked_add(shipped_scale)?,
            ))
        });
    item.total = item
        .total_exact
        .and_then(|(amount, scale)| round_cents(amount, scale));
}

fn valid_number(value: &str) -> bool {
    value
        .trim()
        .parse::<f64>()
        .is_ok_and(|number| number.is_finite())
}

fn normalize_number(value: &str) -> String {
    let Some((amount, scale)) = decimal_parts(value) else {
        return value.into();
    };
    if amount == 0 {
        return "0".into();
    }
    let sign = if amount < 0 { "-" } else { "" };
    let digits = amount.unsigned_abs().to_string();
    if scale <= 0 {
        let Some(zeros) = scale.checked_neg().filter(|zeros| *zeros <= 100) else {
            return value.trim().into();
        };
        return format!("{sign}{digits}{}", "0".repeat(zeros as usize));
    }
    if scale > 100 {
        return value.trim().into();
    }
    let scale = scale as usize;
    let padded = format!(
        "{}{}",
        "0".repeat((scale + 1).saturating_sub(digits.len())),
        digits
    );
    let split = padded.len() - scale;
    let fraction = padded[split..].trim_end_matches('0');
    if fraction.is_empty() {
        format!("{sign}{}", &padded[..split])
    } else {
        format!("{sign}{}.{fraction}", &padded[..split])
    }
}

fn number_message(value: &str) -> &'static str {
    if value
        .bytes()
        .any(|character| !b"0123456789-+eE.".contains(&character))
    {
        "FRM-50016: Legal characters are 0-9 - + E ."
    } else {
        "FRM-50012: Invalid number."
    }
}

impl Session {
    /// Open a database-backed order form with the legacy startup query.
    pub async fn open(url: &str) -> Result<Self> {
        let (client, connection) = tokio_postgres::connect(url, NoTls).await?;
        tokio::spawn(async move {
            if let Err(error) = connection.await {
                eprintln!("PostgreSQL connection: {error}");
            }
        });
        let today: String = client
            .query_one("SELECT current_date::text", &[])
            .await?
            .get(0);
        let products = client.query("SELECT row_to_json(p)::text FROM (SELECT id,name,suggested_whlsl_price FROM s_product ORDER BY id) p", &[]).await?
            .into_iter().map(|row| serde_json::from_str::<Value>(row.get::<_, &str>(0))).collect::<std::result::Result<Vec<_>, _>>()?;
        let mut session = Self {
            client,
            today: today.clone(),
            orders: vec![Order::blank(&today)],
            current: 0,
            deleted: vec![],
            query: false,
            criteria: Fields::new(),
            focus: Some(Focus::header("order-id")),
            immediate: true,
            auto_query: false,
            sort: String::new(),
            alert: None,
            messages: vec![],
            closed: false,
            products,
            transaction_open: false,
            locks: HashSet::new(),
            pending_edit: None,
        };
        session.execute_query().await?;
        session.messages.clear();
        Ok(session)
    }

    fn order(&self) -> &Order {
        &self.orders[self.current]
    }
    fn order_mut(&mut self) -> &mut Order {
        &mut self.orders[self.current]
    }
    fn dirty(&self) -> bool {
        !self.deleted.is_empty() || self.orders.iter().any(Order::changed)
    }
    fn message(&mut self, message: &str) {
        self.messages.push(message.into());
    }

    async fn rollback(&mut self) -> Result<()> {
        if self.transaction_open {
            self.client.batch_execute("ROLLBACK").await?;
        }
        self.transaction_open = false;
        self.locks.clear();
        self.pending_edit = None;
        Ok(())
    }

    async fn reserve(&mut self, command: &Command) -> Result<bool> {
        let original = if let Some(number) = command.line {
            self.order()
                .items
                .get(number.saturating_sub(1))
                .and_then(|item| item.original.clone())
        } else {
            self.order().original.clone()
        };
        let Some(original) = original else {
            return Ok(true);
        };
        let (key, sql, parameters) = if command.line.is_some() {
            let id = text(&original["ord_id"]);
            let item = text(&original["item_id"]);
            (
                format!("item:{id}:{item}"),
                "SELECT to_jsonb(i)=$3::text::jsonb FROM s_item i WHERE ord_id=$1::text::numeric AND item_id=$2::text::numeric FOR UPDATE NOWAIT",
                vec![id, item, original.to_string()],
            )
        } else {
            let id = text(&original["id"]);
            (
                format!("order:{id}"),
                "SELECT to_jsonb(o)=$2::text::jsonb FROM s_ord o WHERE id=$1::text::numeric FOR UPDATE NOWAIT",
                vec![id, original.to_string()],
            )
        };
        if self.locks.contains(&key) {
            return Ok(true);
        }
        if !self.transaction_open {
            self.client.batch_execute("BEGIN").await?;
            self.transaction_open = true;
        }
        self.client
            .batch_execute("SAVEPOINT reserve_record")
            .await?;
        let parameters: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
            parameters.iter().map(|value| value as _).collect();
        match self.client.query_opt(sql, &parameters).await {
            Ok(Some(row)) if row.get::<_, bool>(0) => {
                self.client
                    .batch_execute("RELEASE SAVEPOINT reserve_record")
                    .await?;
                self.locks.insert(key);
                Ok(true)
            }
            Ok(_) => {
                self.client
                    .batch_execute(
                        "ROLLBACK TO SAVEPOINT reserve_record; RELEASE SAVEPOINT reserve_record",
                    )
                    .await?;
                self.message(
                    "FRM-40654: Record has been updated by another user. Re-query to see change.",
                );
                Ok(false)
            }
            Err(error) if error.code().is_some_and(|code| code.code() == "55P03") => {
                self.client
                    .batch_execute(
                        "ROLLBACK TO SAVEPOINT reserve_record; RELEASE SAVEPOINT reserve_record",
                    )
                    .await?;
                self.pending_edit = Some(command.clone());
                self.ask(
                    "Forms",
                    "Could not reserve record (2 tries). Keep trying?",
                    &["Yes", "No"],
                    "reserve",
                    command.line,
                );
                Ok(false)
            }
            Err(error) => {
                self.client
                    .batch_execute(
                        "ROLLBACK TO SAVEPOINT reserve_record; RELEASE SAVEPOINT reserve_record",
                    )
                    .await?;
                Err(error.into())
            }
        }
    }

    /// Serialize displayed values and UI states; line summaries are calculated on leave.
    pub fn screen(&mut self) -> Value {
        let order = self.order();
        let mut lines: Vec<Fields> = order
            .items
            .iter()
            .map(|item| {
                let mut fields = item.fields.clone();
                if let Ok(price) = get(&fields, "price").parse::<f64>() {
                    fields.insert("price".into(), format!("{price:.2}"));
                }
                fields.insert(
                    "item-total".into(),
                    item.total.map(money).unwrap_or_default(),
                );
                fields
            })
            .collect();
        if self.query {
            lines = vec![Fields::new()];
        }
        let disabled: Vec<&str> = if self.query {
            if self.immediate {
                vec!["auto-query", "customer-name", "rep-name"]
            } else {
                vec!["customer-name", "rep-name"]
            }
        } else if self.immediate {
            vec!["auto-query"]
        } else {
            vec![]
        };
        json!({
            "order": if self.query { &self.criteria } else { &order.fields },
            "lines": lines,
            "total": if self.query { None } else { order.total().map(money) },
            "focus": self.focus,
            "mode": if self.query { "query" } else { "normal" },
            "disabled": disabled, "required": ["customer-id"],
            "options": {"immediate": self.immediate, "auto-query": self.auto_query},
            "sort": if self.sort.is_empty() { "id" } else { &self.sort },
            "alert": self.alert, "closed": self.closed,
            "messages": self.messages, "products": self.products,
        })
    }

    async fn load_details(&mut self) -> Result<()> {
        if self.order().details_loaded {
            return Ok(());
        }
        let id = get(&self.order().fields, "order-id");
        let rows = self.client.query("SELECT row_to_json(i)::text, p.name FROM s_item i JOIN s_product p ON p.id=i.product_id WHERE ord_id=$1::text::numeric ORDER BY item_id", &[&id]).await?;
        let mut items = Vec::new();
        for row in rows {
            let doc: Value = serde_json::from_str(row.get(0))?;
            let mut item = Item {
                fields: fields(&doc, LINE),
                original: Some(doc),
                ..Item::default()
            };
            item.fields.insert("description".into(), row.get(1));
            item.validated_product = get(&item.fields, "product-id");
            if let Some(value) = cents(&get(&item.fields, "price")) {
                item.fields.insert("price".into(), money(value));
            }
            calculate(&mut item);
            items.push(item);
        }
        if items.is_empty() {
            items.push(Item::default());
        }
        self.order_mut().items = items;
        self.order_mut().details_loaded = true;
        Ok(())
    }

    async fn execute_query(&mut self) -> Result<()> {
        self.rollback().await?;
        if !self.query {
            self.criteria.clear();
        }
        let mut conditions = Vec::new();
        let mut values = Vec::new();
        for (item, column) in HEADER {
            // PRE-QUERY replaces customer criteria with GLOBAL.customer_id (empty in this entrypoint).
            if *item == "customer-id" {
                continue;
            }
            let value = get(&self.criteria, item);
            if !value.is_empty() {
                let operators = [">=", "<=", "!=", "<>", ">", "<", "="];
                let operator = operators
                    .iter()
                    .find(|operator| value.starts_with(**operator))
                    .copied()
                    .unwrap_or("=");
                let operand = value
                    .strip_prefix(operator)
                    .unwrap_or(&value)
                    .trim()
                    .to_string();
                let cast = match *column {
                    "id" | "sales_rep_id" => "numeric",
                    "date_ordered" | "date_shipped" => "date",
                    _ => "text",
                };
                if cast == "numeric" && !valid_number(&operand) {
                    self.message(number_message(&operand));
                    return Ok(());
                }
                values.push(operand);
                conditions.push(format!(
                    "o.{column} {operator} ${}::text::{cast}",
                    values.len()
                ));
            }
        }
        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", conditions.join(" AND "))
        };
        let sort = match self.sort.as_str() {
            "id" | "customer_id" | "sales_rep_id" => format!(" ORDER BY o.{},o.id", self.sort),
            _ => String::new(),
        };
        let sql = format!(
            "SELECT row_to_json(o)::text,c.name,e.last_name FROM s_ord o JOIN s_customer c ON c.id=o.customer_id JOIN s_emp e ON e.id=c.sales_rep_id{where_clause}{sort}"
        );
        let parameters: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
            values.iter().map(|value| value as _).collect();
        let rows = self.client.query(&sql, &parameters).await?;
        self.orders.clear();
        for row in rows {
            let doc: Value = serde_json::from_str(row.get(0))?;
            let mut order = Order::blank(&self.today);
            order.fields = fields(&doc, HEADER);
            order.fields.insert("customer-name".into(), row.get(1));
            order.fields.insert("rep-name".into(), row.get(2));
            order.original = Some(doc);
            order.validated_customer = get(&order.fields, "customer-id");
            order.details_loaded = false;
            self.orders.push(order);
        }
        self.current = 0;
        self.query = false;
        self.deleted.clear();
        self.focus = Some(Focus::header("order-id"));
        if self.orders.is_empty() {
            self.orders.push(Order::blank(&self.today));
            self.query = true;
            self.message("FRM-40301: Query caused no records to be retrieved. Re-enter.");
        } else if self.immediate {
            self.load_details().await?;
        }
        Ok(())
    }

    fn ask(
        &mut self,
        title: &str,
        text: &str,
        buttons: &[&str],
        continuation: &str,
        line: Option<usize>,
    ) {
        self.alert = Some(Alert {
            title: title.into(),
            text: text.into(),
            buttons: buttons.iter().map(|button| (*button).into()).collect(),
            continuation: continuation.into(),
            line,
        });
    }

    fn reset(&mut self) {
        self.orders = vec![Order::blank(&self.today)];
        self.current = 0;
        self.deleted.clear();
        self.query = false;
        self.criteria.clear();
        self.focus = Some(Focus::header("order-id"));
    }

    async fn remove_current_order(&mut self, queue_deletion: bool) -> Result<()> {
        if queue_deletion && let Some(original) = self.order().original.clone() {
            self.deleted.push(original);
        }
        self.orders.remove(self.current);
        if self.current > 0 {
            self.current -= 1;
        }
        if self.orders.is_empty() {
            self.orders.push(Order::blank(&self.today));
            self.current = 0;
        }
        self.focus = Some(Focus::header("order-id"));
        if self.immediate {
            self.load_details().await?;
        }
        Ok(())
    }

    fn enter_query(&mut self) {
        if self.query {
            return;
        }
        self.reset();
        self.query = true;
    }

    async fn validate_current(&mut self) -> Result<bool> {
        if self.query {
            return Ok(true);
        }
        let Some(focus) = self.focus.clone() else {
            return Ok(true);
        };
        if let Some(number) = focus.line {
            let index = number.saturating_sub(1);
            let Some(item) = self.order().items.get(index) else {
                return Ok(true);
            };
            let value = get(&item.fields, &focus.item);
            if focus.item == "product-id" && value.is_empty() && item.dirty {
                self.message("FRM-40202: Field must be entered.");
                return Ok(false);
            }
            if !value.is_empty()
                && ["item-id", "product-id", "price", "quantity", "shipped"]
                    .contains(&focus.item.as_str())
                && !valid_number(&value)
            {
                self.message(number_message(&value));
                return Ok(false);
            }
            if focus.item == "product-id"
                && (value != item.validated_product || (value.is_empty() && item.dirty))
            {
                let row = self
                    .client
                    .query_opt(
                        "SELECT name,suggested_whlsl_price::text FROM s_product WHERE id=$1::text::numeric",
                        &[&value],
                    )
                    .await?;
                if let Some(row) = row {
                    let item = &mut self.order_mut().items[index];
                    item.fields.insert("description".into(), row.get(0));
                    let price: Option<String> = row.get(1);
                    item.fields.insert(
                        "price".into(),
                        price
                            .and_then(|price| cents(&price))
                            .map(money)
                            .unwrap_or_default(),
                    );
                    item.validated_product = value.clone();
                } else {
                    self.message(if value.is_empty() {
                        "FRM-40202: Field must be entered."
                    } else {
                        "Invalid Product Id!"
                    });
                    return Ok(false);
                }
            }
            if !value.is_empty()
                && ["item-id", "product-id", "price", "quantity", "shipped"]
                    .contains(&focus.item.as_str())
            {
                let valid = valid_number(&value);
                if !valid {
                    self.message(number_message(&value));
                    return Ok(false);
                }
            }
            let item = &mut self.order_mut().items[index];
            calculate(item);
        } else {
            let value = get(&self.order().fields, &focus.item);
            if focus.item == "customer-id" && value.is_empty() {
                self.message("FRM-40202: Field must be entered.");
                return Ok(false);
            }
            if !value.is_empty()
                && ["order-id", "customer-id", "rep-id"].contains(&focus.item.as_str())
                && !valid_number(&value)
            {
                self.message(number_message(&value));
                return Ok(false);
            }
            if focus.item == "customer-id" && value != self.order().validated_customer {
                let row = self.client.query_opt("SELECT c.name,c.sales_rep_id::text,e.last_name FROM s_customer c JOIN s_emp e ON e.id=c.sales_rep_id WHERE c.id=$1::text::numeric", &[&value]).await?;
                if let Some(row) = row {
                    let order = self.order_mut();
                    order.fields.insert("customer-name".into(), row.get(0));
                    order.fields.insert("rep-id".into(), row.get(1));
                    order.fields.insert("rep-name".into(), row.get(2));
                    order.validated_customer = value.clone();
                    order.dirty = true;
                } else {
                    self.message(if value.is_empty() {
                        "FRM-40202: Field must be entered."
                    } else {
                        "Invalid Customer Id!"
                    });
                    return Ok(false);
                }
            } else if focus.item == "customer-id" && value.is_empty() {
                self.message("FRM-40202: Field must be entered.");
                return Ok(false);
            }
            if ["order-id", "customer-id", "rep-id"].contains(&focus.item.as_str())
                && !value.is_empty()
                && !valid_number(&value)
            {
                self.message(number_message(&value));
                return Ok(false);
            }
        }
        Ok(true)
    }

    async fn move_focus(&mut self, target: Focus) -> Result<bool> {
        if self
            .focus
            .as_ref()
            .is_some_and(|focus| focus.item == target.item && focus.line == target.line)
        {
            return Ok(true);
        }
        if !self.validate_current().await? {
            return Ok(false);
        }
        if target.line.is_some() {
            if !self.validate_header() {
                return Ok(false);
            }
            if self.immediate || self.auto_query {
                self.load_details().await?;
            }
            while self.order().items.len() < target.line.unwrap_or(1) {
                self.order_mut().items.push(Item::default());
            }
        }
        self.focus = Some(target);
        Ok(true)
    }

    fn validate_header(&mut self) -> bool {
        let ordered = get(&self.order().fields, "date-ordered");
        let shipped = get(&self.order().fields, "date-shipped");
        if !ordered.is_empty() && !shipped.is_empty() && shipped < ordered {
            self.message("Ship date is before order date!");
            self.focus = Some(Focus::header("order-id"));
            false
        } else {
            true
        }
    }

    /// Execute one gesture and preserve focus across toolbar commands.
    pub async fn apply(&mut self, mut command: Command) -> Result<()> {
        self.messages.clear();
        if self.closed {
            return Ok(());
        }
        if command.action == "answer" {
            return self.answer(&command.value).await;
        }
        if self.alert.is_some() {
            return Ok(());
        }
        if command.action == "type" {
            // A refused field leave keeps the cursor in the invalid field, including subsequent typing.
            if let Some(focus) = &self.focus {
                command.item = focus.item.clone();
                command.line = focus.line;
            }
            if !command.value.is_empty()
                && [
                    "order-id",
                    "customer-id",
                    "rep-id",
                    "item-id",
                    "product-id",
                    "price",
                    "quantity",
                    "shipped",
                ]
                .contains(&command.item.as_str())
                && valid_number(&command.value)
            {
                command.value = normalize_number(&command.value);
            }
        }
        if self.query
            && [
                "save",
                "cancel",
                "new-order",
                "duplicate-order",
                "next-order",
                "previous-order",
                "delete-order",
                "delete-line",
                "duplicate-line",
                "add-line",
            ]
            .contains(&command.action.as_str())
        {
            self.message(QUERY_KEY_MESSAGE);
            return Ok(());
        }
        if self.query && command.action == "clear" {
            self.criteria.clear();
            self.focus = Some(Focus::header("order-id"));
            return Ok(());
        }
        if self.query && command.action == "exit" {
            return Ok(());
        }
        if [
            "new-order",
            "duplicate-order",
            "next-order",
            "previous-order",
        ]
        .contains(&command.action.as_str())
            && !(command.action == "previous-order" && self.current == 0)
            && (self.order().items.iter().any(|item| item.dirty)
                || !self.order().removed.is_empty())
        {
            if self.move_focus(Focus::header("order-id")).await? && self.validate_header() {
                self.ask(
                    "Forms",
                    "Do you want to save the changes you have made?",
                    &["Yes", "No", "Cancel"],
                    &format!("navigate:{}", command.action),
                    None,
                );
            }
            return Ok(());
        }
        let writes_existing = match command.action.as_str() {
            "type" => {
                !self.query
                    && if let Some(number) = command.line {
                        self.order()
                            .items
                            .get(number.saturating_sub(1))
                            .is_some_and(|item| {
                                get(&item.fields, &command.item) != command.value
                                    && !(command.item == "shipped" && item.original.is_some())
                            })
                    } else {
                        get(&self.order().fields, &command.item) != command.value
                    }
            }
            "pick" | "payment" => !self.query,
            "toggle" => !self.query && command.item == "order-filled",
            _ => false,
        };
        if writes_existing && !self.reserve(&command).await? {
            return Ok(());
        }
        match command.action.as_str() {
            "click" => {
                let target = Focus {
                    item: command.item,
                    line: command.line,
                };
                self.move_focus(target).await?;
            }
            "type" => {
                if self.query {
                    self.criteria.insert(command.item, command.value);
                } else if let Some(number) = command.line {
                    if let Some(item) = self.order_mut().items.get_mut(number.saturating_sub(1)) {
                        // QUANTITY_SHIPPED is insertable but not updateable on a queried line.
                        if command.item == "shipped" && item.original.is_some() {
                            self.message("FRM-40200: Field is protected against update.");
                        } else if get(&item.fields, &command.item) != command.value {
                            item.fields.insert(command.item, command.value);
                            item.dirty = true;
                        }
                    }
                } else if get(&self.order().fields, &command.item) != command.value {
                    self.order_mut().fields.insert(command.item, command.value);
                    self.order_mut().dirty = true;
                }
            }
            "next-item" => {
                if let Some(focus) = self.focus.clone()
                    && let Some(number) = focus.line
                {
                    let sequence = ["item-id", "product-id", "price", "quantity", "shipped"];
                    let position = sequence
                        .iter()
                        .position(|item| *item == focus.item)
                        .unwrap_or(0);
                    let target = if position + 1 < sequence.len() {
                        Focus::line(sequence[position + 1], number)
                    } else {
                        Focus::line("item-id", number + 1)
                    };
                    self.move_focus(target).await?;
                } else {
                    self.message("Function Not Allowed");
                }
            }
            "cancel" => self.message("Function Not Allowed"),
            "sort" => {
                if ["id", "customer_id", "sales_rep_id"].contains(&command.value.as_str()) {
                    self.sort = command.value;
                }
            }
            "toggle" => match command.item.as_str() {
                "immediate" => self.immediate = !self.immediate,
                "auto-query" if !self.immediate => self.auto_query = !self.auto_query,
                "order-filled" if self.move_focus(Focus::header("order-filled")).await? => {
                    let value = if get(&self.order().fields, "order-filled") == "Y" {
                        "N"
                    } else {
                        "Y"
                    };
                    if self.query {
                        self.criteria.insert("order-filled".into(), value.into());
                    } else {
                        self.order_mut()
                            .fields
                            .insert("order-filled".into(), value.into());
                        self.order_mut().dirty = true;
                    }
                }
                _ => (),
            },
            "payment" => {
                if self.move_focus(Focus::header("payment-type")).await? {
                    if self.query {
                        self.criteria.insert("payment-type".into(), command.value);
                    } else if ["", "CASH", "CREDIT"].contains(&command.value.as_str()) {
                        self.order_mut()
                            .fields
                            .insert("payment-type".into(), command.value.clone());
                        self.order_mut().dirty = true;
                        if command.value == "CREDIT" {
                            let customer = get(&self.order().fields, "customer-id");
                            if let Some(row) = self
                                .client
                                .query_opt(
                                    "SELECT credit_rating FROM s_customer WHERE id::text=$1",
                                    &[&customer],
                                )
                                .await?
                            {
                                let rating: Option<String> = row.get(0);
                                if rating
                                    .is_some_and(|rating| rating != "GOOD" && rating != "EXCELLENT")
                                {
                                    self.order_mut()
                                        .fields
                                        .insert("payment-type".into(), "CASH".into());
                                    self.ask(
                                        "Payment Type",
                                        "This customer must pay cash!",
                                        &["OK"],
                                        "payment",
                                        None,
                                    );
                                }
                            }
                        }
                    }
                }
            }
            "pick" => {
                let number = command.line.unwrap_or(1);
                if self.move_focus(Focus::line("product-id", number)).await? {
                    let index = number.saturating_sub(1);
                    self.order_mut().items[index]
                        .fields
                        .insert("product-id".into(), command.value);
                    self.order_mut().items[index].dirty = true;
                    if self.validate_current().await? {
                        self.focus = Some(Focus::line("price", number));
                    }
                }
            }
            "add-line" => {
                if self.validate_current().await? && self.validate_header() {
                    self.load_details().await?;
                    self.order_mut().items.push(Item::default());
                    self.focus = Some(Focus::line("item-id", self.order().items.len()));
                }
            }
            "save" => {
                self.save().await?;
            }
            "enter-query" | "execute-query" | "clear" => {
                if self.dirty() {
                    self.ask(
                        "Forms",
                        "Do you want to save the changes you have made?",
                        &["Yes", "No", "Cancel"],
                        &command.action,
                        None,
                    );
                } else {
                    match command.action.as_str() {
                        "enter-query" => {
                            self.rollback().await?;
                            self.enter_query();
                        }
                        "execute-query" => self.execute_query().await?,
                        _ => {
                            self.rollback().await?;
                            self.reset();
                        }
                    }
                }
            }
            "new-order" | "duplicate-order" => {
                if self.validate_current().await? && self.validate_header() {
                    let reuse_empty = self.order().original.is_none() && !self.order().changed();
                    if reuse_empty {
                        self.message("FRM-40102: Record must be entered or deleted first.");
                        if command.action == "new-order" {
                            return Ok(());
                        }
                        if self.current == 0 {
                            self.message("FRM-41803: No previous record to copy value from.");
                            self.order_mut().dirty = true;
                            self.ask(
                                "Forms",
                                "FRM-40102: Record must be entered or deleted first.",
                                &["OK"],
                                "payment",
                                None,
                            );
                            return Ok(());
                        }
                    }
                    let mut order = if command.action == "duplicate-order" {
                        self.orders[if reuse_empty {
                            self.current - 1
                        } else {
                            self.current
                        }]
                        .clone()
                    } else {
                        Order::blank(&self.today)
                    };
                    if command.action == "duplicate-order" {
                        order.fields.remove("order-id");
                        order.original = None;
                        order.items = vec![Item::default()];
                        order.removed.clear();
                        order.details_loaded = true;
                        order.dirty = true;
                    }
                    if reuse_empty {
                        self.orders[self.current] = order;
                    } else {
                        self.current += 1;
                        self.orders.insert(self.current, order);
                    }
                    self.focus = Some(Focus::header("order-id"));
                }
            }
            "next-order" | "previous-order" => {
                if self.validate_current().await? && self.validate_header() {
                    if command.action == "next-order" {
                        if self.order().original.is_none() && !self.order().changed() {
                            self.message("FRM-40102: Record must be entered or deleted first.");
                            return Ok(());
                        }
                        if self.current + 1 == self.orders.len() {
                            self.orders.push(Order::blank(&self.today));
                            self.message("FRM-40352: Last record of query retrieved.");
                        }
                        self.current += 1;
                    } else if self.current > 0 {
                        self.current -= 1;
                    } else {
                        self.message("FRM-40100: At first record.");
                    }
                    self.focus = Some(Focus::header("order-id"));
                    if self.immediate {
                        self.load_details().await?;
                    }
                }
            }
            "delete-order" => {
                if self.validate_current().await? {
                    self.focus = Some(Focus::header("order-id"));
                    self.ask(
                        "Forms",
                        "Do you want to delete this order?",
                        &["Yes", "No"],
                        "delete-order",
                        None,
                    );
                }
            }
            "delete-line" => {
                let number = command.line.unwrap_or(1);
                if self.move_focus(Focus::line("product-id", number)).await? {
                    self.ask(
                        "Forms",
                        "Do you want to delete this line order?",
                        &["Yes", "No"],
                        "delete-line",
                        Some(number),
                    );
                }
            }
            "duplicate-line" => {
                let number = command.line.unwrap_or(1);
                if self.validate_current().await? {
                    self.load_details().await?;
                    if let Some(previous) = self.order().items.get(number.saturating_sub(1)) {
                        let mut item = previous.clone();
                        for field in ["item-id", "product-id", "description", "price"] {
                            item.fields.remove(field);
                        }
                        item.original = None;
                        item.validated_product.clear();
                        item.total = None;
                        item.total_exact = None;
                        item.dirty = true;
                        self.order_mut().items.insert(number, item);
                        self.focus = Some(Focus::line("item-id", number + 1));
                    }
                }
            }
            "exit" => {
                if !self.validate_current().await? || !self.validate_header() {
                    return Ok(());
                }
                // The exit control's focus is outside the data blocks.
                self.auto_query = true;
                self.ask(
                    "Question",
                    "Do you really want to leave the form?",
                    &["Yes", "No"],
                    "exit",
                    None,
                );
            }
            _ => self.message("Function Not Allowed"),
        }
        Ok(())
    }

    async fn answer(&mut self, button: &str) -> Result<()> {
        let Some(alert) = self.alert.take() else {
            self.message("Function Not Allowed");
            return Ok(());
        };
        if !alert.buttons.iter().any(|choice| choice == button) {
            self.alert = Some(alert);
            return Ok(());
        }
        match alert.continuation.as_str() {
            continuation if continuation.starts_with("navigate:") => {
                let action = continuation.trim_start_matches("navigate:");
                if button == "Cancel" {
                    if action == "duplicate-order" {
                        self.order_mut().fields.remove("order-id");
                        self.order_mut().dirty = true;
                        self.message("FRM-41803: No previous record to copy value from.");
                    }
                    return Ok(());
                }
                if button == "Yes" {
                    if !self.save().await? {
                        return Ok(());
                    }
                } else {
                    // Relation coordination clears the detail block, not uncommitted header edits.
                    self.rollback().await?;
                    self.order_mut().items = vec![Item::default()];
                    self.order_mut().removed.clear();
                    self.order_mut().details_loaded = false;
                    self.load_details().await?;
                }
                let messages = self.messages.clone();
                Box::pin(self.apply(Command {
                    action: action.into(),
                    item: String::new(),
                    value: String::new(),
                    line: None,
                }))
                .await?;
                self.messages.splice(0..0, messages);
            }
            "reserve" => {
                let command = self.pending_edit.take();
                if button == "Yes" {
                    if let Some(command) = command {
                        Box::pin(self.apply(command)).await?;
                    }
                } else {
                    self.message(
                        "FRM-40501: ORACLE error: unable to reserve record for update or delete.",
                    );
                }
            }
            "payment" => (),
            "delete-new" => {
                if button != "Cancel" {
                    if button == "Yes" && !self.save().await? {
                        return Ok(());
                    }
                    self.remove_current_order(false).await?;
                }
            }
            "finish-close" => {
                self.rollback().await?;
                self.closed = true;
            }
            "close" => {
                if button == "Cancel" {
                    self.focus = None;
                } else if button == "No" {
                    self.rollback().await?;
                    self.closed = true;
                } else if self.save().await? {
                    let text = self.messages.last().cloned().unwrap_or_default();
                    self.ask("Forms", &text, &["OK"], "finish-close", None);
                }
            }
            "exit" => {
                if button == "Yes" {
                    if self.dirty() {
                        self.ask(
                            "Forms",
                            "Do you want to save the changes you have made?",
                            &["Yes", "No", "Cancel"],
                            "close",
                            None,
                        );
                    } else {
                        self.rollback().await?;
                        self.closed = true;
                    }
                } else {
                    self.focus = None;
                }
            }
            "delete-order" => {
                if button == "Yes" {
                    let command = Command {
                        action: "delete-order".into(),
                        item: String::new(),
                        value: String::new(),
                        line: None,
                    };
                    if !self.reserve(&command).await? {
                        return Ok(());
                    }
                    let id = get(&self.order().fields, "order-id");
                    let exists = if !id.is_empty() {
                        self.client.query_one("SELECT EXISTS(SELECT 1 FROM s_item WHERE ord_id=$1::text::numeric)", &[&id]).await?.get::<_, bool>(0)
                    } else {
                        false
                    };
                    if exists {
                        self.message(
                            "Cannot delete master record when matching detail records exist.",
                        );
                    } else if self.order().original.is_none()
                        && self.order().items.iter().any(|item| item.dirty)
                    {
                        self.ask(
                            "Forms",
                            "Do you want to save the changes you have made?",
                            &["Yes", "No", "Cancel"],
                            "delete-new",
                            None,
                        );
                    } else {
                        self.remove_current_order(true).await?;
                    }
                }
            }
            "delete-line" => {
                if button == "Yes" {
                    let command = Command {
                        action: "delete-line".into(),
                        item: String::new(),
                        value: String::new(),
                        line: alert.line,
                    };
                    if !self.reserve(&command).await? {
                        return Ok(());
                    }
                    let index = alert.line.unwrap_or(1).saturating_sub(1);
                    if index < self.order().items.len() {
                        let item = self.order_mut().items.remove(index);
                        if let Some(original) = item.original {
                            self.order_mut().removed.push(original);
                        }
                        if self.order().items.is_empty() {
                            self.order_mut().items.push(Item::default());
                        }
                        let number = (index + 1).min(self.order().items.len());
                        self.focus = Some(Focus::line("item-id", number));
                    }
                }
            }
            continuation => {
                if button != "Cancel" {
                    if button == "Yes" && !self.save().await? {
                        return Ok(());
                    }
                    match continuation {
                        "enter-query" => {
                            self.rollback().await?;
                            self.enter_query();
                        }
                        "execute-query" => self.execute_query().await?,
                        _ => {
                            self.rollback().await?;
                            self.reset();
                        }
                    }
                }
            }
        }
        Ok(())
    }

    async fn save(&mut self) -> Result<bool> {
        if self.query {
            self.message(QUERY_KEY_MESSAGE);
            return Ok(false);
        }
        if !self.validate_current().await? || !self.validate_header() {
            return Ok(false);
        }
        if !self.dirty() {
            self.message("FRM-40401: No changes to save.");
            return Ok(true);
        }
        for (index, order) in self.orders.iter().enumerate() {
            if order.changed() && get(&order.fields, "customer-id").is_empty() {
                self.current = index;
                self.focus = Some(Focus::header("customer-id"));
                self.message("FRM-40202: Field must be entered.");
                return Ok(false);
            }
            if let Some(index_item) = order
                .items
                .iter()
                .position(|item| item.dirty && get(&item.fields, "product-id").is_empty())
            {
                self.current = index;
                self.focus = Some(Focus::line("product-id", index_item + 1));
                self.message("FRM-40202: Field must be entered.");
                return Ok(false);
            }
        }
        let current = self.current;
        let focus = self.focus.clone();
        for index in 0..self.orders.len() {
            if !self.orders[index].changed() {
                continue;
            }
            self.current = index;
            if get(&self.order().fields, "customer-id") != self.order().validated_customer {
                self.focus = Some(Focus::header("customer-id"));
                if !self.validate_current().await? {
                    return Ok(false);
                }
            }
            for item in ["order-id", "customer-id", "rep-id"] {
                let value = get(&self.order().fields, item);
                if !value.is_empty() && !valid_number(&value) {
                    self.focus = Some(Focus::header(item));
                    self.message(number_message(&value));
                    return Ok(false);
                }
            }
            if !self.validate_header() {
                return Ok(false);
            }
            for line in 0..self.order().items.len() {
                if !self.order().items[line].dirty {
                    continue;
                }
                if get(&self.order().items[line].fields, "product-id")
                    != self.order().items[line].validated_product
                {
                    self.focus = Some(Focus::line("product-id", line + 1));
                    if !self.validate_current().await? {
                        return Ok(false);
                    }
                }
                for item in ["item-id", "product-id", "price", "quantity", "shipped"] {
                    let value = get(&self.order().items[line].fields, item);
                    if !value.is_empty() && !valid_number(&value) {
                        self.focus = Some(Focus::line(item, line + 1));
                        self.message(number_message(&value));
                        return Ok(false);
                    }
                }
                calculate(&mut self.order_mut().items[line]);
            }
        }
        self.current = current;
        self.focus = focus;
        let mut pending = self.orders.clone();
        let deleted = self.deleted.clone();
        if !self.transaction_open {
            self.client.batch_execute("BEGIN").await?;
            self.transaction_open = true;
        }
        self.client.batch_execute("SAVEPOINT commit_form").await?;
        let mut location = None;
        let result = persist(&self.client, &mut pending, &deleted, &mut location).await;
        match result {
            Ok(count) => {
                self.client.batch_execute("COMMIT").await?;
                self.transaction_open = false;
                self.locks.clear();
                self.orders = pending;
                self.deleted.clear();
                self.message(&format!(
                    "FRM-40400: Transaction complete: {count} records applied and saved."
                ));
                Ok(true)
            }
            Err(error) => {
                self.client
                    .batch_execute(
                        "ROLLBACK TO SAVEPOINT commit_form; RELEASE SAVEPOINT commit_form",
                    )
                    .await?;
                // PRE-INSERT assignments remain displayed after rollback, but are not committed rows.
                for (order, attempted) in self.orders.iter_mut().zip(&pending) {
                    if order.original.is_none()
                        && let Some(id) = attempted.fields.get("order-id")
                    {
                        order.fields.insert("order-id".into(), id.clone());
                    }
                    for (item, attempted_item) in order.items.iter_mut().zip(&attempted.items) {
                        if item.original.is_none()
                            && let Some(id) = attempted_item.fields.get("item-id")
                        {
                            item.fields.insert("item-id".into(), id.clone());
                        }
                    }
                }
                let message = error.to_string();
                if message == "changed" {
                    self.message("FRM-40654: Record has been updated by another user. Re-query to see change.");
                } else if let Some(error) = error.downcast_ref::<tokio_postgres::Error>() {
                    let code = error.code().map(|code| code.code()).unwrap_or("");
                    let inserting = location.is_some_and(|(_, _, inserting)| inserting);
                    if let Some((order, line, _)) = location {
                        self.current = order;
                        self.focus = Some(if let Some(line) = line {
                            Focus::line("item-id", line + 1)
                        } else {
                            Focus::header("order-id")
                        });
                    }
                    self.message(match code {
                        "55P03" => "FRM-40501: ORACLE error: unable to reserve record for update or delete.",
                        "23505" | "23503" | "23514" | "23502" | "22003" if inserting => "FRM-40508: ORACLE error: unable to INSERT record.",
                        "23505" | "23503" | "23514" | "23502" | "22003" => "FRM-40509: ORACLE error: unable to UPDATE record.",
                        "22007" | "22008" => "FRM-40209: Field must be of form DD-MON-RR.",
                        _ => "FRM-40508: ORACLE error: unable to INSERT record.",
                    });
                } else {
                    return Err(error);
                }
                Ok(false)
            }
        }
    }
}

async fn check_order(transaction: &Client, original: &Value) -> Result<()> {
    let id = text(&original["id"]);
    let row = transaction.query_opt("SELECT to_jsonb(o)=$2::text::jsonb FROM s_ord o WHERE id=$1::text::numeric FOR UPDATE NOWAIT", &[&id, &original.to_string()]).await?;
    if !row.is_some_and(|row| row.get::<_, bool>(0)) {
        return Err("changed".into());
    }
    Ok(())
}

async fn check_item(transaction: &Client, original: &Value) -> Result<()> {
    let id = text(&original["ord_id"]);
    let item_id = text(&original["item_id"]);
    let row = transaction.query_opt("SELECT to_jsonb(i)=$3::text::jsonb FROM s_item i WHERE ord_id=$1::text::numeric AND item_id=$2::text::numeric FOR UPDATE NOWAIT", &[&id, &item_id, &original.to_string()]).await?;
    if !row.is_some_and(|row| row.get::<_, bool>(0)) {
        return Err("changed".into());
    }
    Ok(())
}

async fn persist(
    transaction: &Client,
    orders: &mut [Order],
    deleted: &[Value],
    location: &mut Option<(usize, Option<usize>, bool)>,
) -> Result<usize> {
    let mut count = 0;
    for original in deleted {
        check_order(transaction, original).await?;
        let id = text(&original["id"]);
        transaction
            .execute("DELETE FROM s_ord WHERE id=$1::text::numeric", &[&id])
            .await?;
        count += 1;
    }
    for (order_index, order) in orders
        .iter_mut()
        .enumerate()
        .filter(|(_, order)| order.changed())
    {
        *location = Some((order_index, None, order.original.is_none()));
        if order.dirty
            && let Some(original) = &order.original
        {
            check_order(transaction, original).await?;
        }
        if order.original.is_none() {
            let id: String = transaction
                .query_one("SELECT nextval('s_ord_id')::text", &[])
                .await?
                .get(0);
            order.fields.insert("order-id".into(), id);
            let doc = order.document().to_string();
            let row = transaction.query_one("INSERT INTO s_ord SELECT (json_populate_record(NULL::s_ord,$1::text::json)).* RETURNING row_to_json(s_ord)::text", &[&doc]).await?;
            order.original = Some(serde_json::from_str(row.get(0))?);
            count += 1;
        } else if order.dirty {
            let doc = order.document().to_string();
            let id = text(&order.original.as_ref().unwrap()["id"]);
            let row = transaction.query_one("UPDATE s_ord SET (id,customer_id,date_ordered,date_shipped,sales_rep_id,total,payment_type,order_filled)=(SELECT id,customer_id,date_ordered,date_shipped,sales_rep_id,total,payment_type,order_filled FROM json_populate_record(NULL::s_ord,$1::text::json)) WHERE id=$2::text::numeric RETURNING row_to_json(s_ord)::text", &[&doc, &id]).await?;
            order.original = Some(serde_json::from_str(row.get(0))?);
            count += 1;
        }
        let id = get(&order.fields, "order-id");
        for original in &order.removed {
            check_item(transaction, original).await?;
            let item_id = text(&original["item_id"]);
            transaction.execute("DELETE FROM s_item WHERE ord_id=$1::text::numeric AND item_id=$2::text::numeric", &[&id, &item_id]).await?;
            count += 1;
        }
        for (item_index, item) in order
            .items
            .iter_mut()
            .enumerate()
            .filter(|(_, item)| item.dirty)
        {
            *location = Some((order_index, Some(item_index), item.original.is_none()));
            let mut doc = document(&item.fields, LINE);
            doc["ord_id"] = json!(id);
            if let Some(original) = &item.original {
                check_item(transaction, original).await?;
                let item_id = text(&original["item_id"]);
                let row = transaction.query_one("UPDATE s_item SET (item_id,product_id,price,quantity,quantity_shipped)=(SELECT item_id,product_id,price,quantity,quantity_shipped FROM json_populate_record(NULL::s_item,$1::text::json)) WHERE ord_id=$2::text::numeric AND item_id=$3::text::numeric RETURNING row_to_json(s_item)::text", &[&doc.to_string(), &id, &item_id]).await?;
                item.original = Some(serde_json::from_str(row.get(0))?);
            } else {
                let item_id: String = transaction.query_one("SELECT (COALESCE(MAX(item_id),0)+1)::text FROM s_item WHERE ord_id=$1::text::numeric", &[&id]).await?.get(0);
                item.fields.insert("item-id".into(), item_id.clone());
                doc["item_id"] = json!(item_id);
                let row = transaction.query_one("INSERT INTO s_item SELECT (json_populate_record(NULL::s_item,$1::text::json)).* RETURNING row_to_json(s_item)::text", &[&doc.to_string()]).await?;
                item.original = Some(serde_json::from_str(row.get(0))?);
            }
            calculate(item);
            item.dirty = false;
            count += 1;
        }
        order.dirty = false;
        order.removed.clear();
    }
    Ok(count)
}
