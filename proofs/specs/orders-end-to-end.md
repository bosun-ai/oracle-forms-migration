```parity
proof orders-end-to-end.explicit-query-edit {
  when entered = enter_query()
  when criteria = type(item = "order-id", value = 101)
  when queried = execute_query()
  when changed = type(item = "customer-id", value = 201)
  when left = click(item = "date-shipped")
  when saved = save()
  then {
    assert steps.entered.mode == "query"
    assert steps.queried.order["order-id"] == 101
    assert steps.changed.order["customer-name"] == "Kam's Sporting Goods"
    assert steps.left.order["customer-name"] == "Unisports"
    assert result.order["rep-id"] == 12
    assert result.messages == ["FRM-40400: Transaction complete: 1 records applied and saved."]
  }
}
```

```parity
proof orders-end-to-end.coordination-sort {
  when entered = enter_query()
  when criteria = type(item = "order-id", value = 102)
  when id_sort = sort_by(field = "id")
  when queried = execute_query()
  when deferred = toggle(item = "immediate")
  when auto_on = toggle(item = "auto-query")
  when next = next_order()
  when detail = click(item = "product-id", line = 1)
  when auto_off = toggle(item = "auto-query")
  when immediate = toggle(item = "immediate")
  when customer_sort = sort_by(field = "customer_id")
  when rep_sort = sort_by(field = "sales_rep_id")
  when previous = previous_order()
  then {
    assert steps.queried.order["order-id"] == 102
    assert steps.deferred.options.immediate == false
    assert steps.auto_on.options["auto-query"] == true
    assert steps.immediate.disabled == ["auto-query"]
    assert result.order["order-id"] == 102
  }
}
```

```parity
proof orders-end-to-end.header-refusals {
  when entered = enter_query()
  when criteria = type(item = "order-id", value = 103)
  when queried = execute_query()
  when invalid = type(item = "customer-id", value = 999999)
  when refused = click(item = "date-shipped")
  when tabbed = next_item()
  when repaired = type(item = "customer-id", value = 202)
  when left = click(item = "date-shipped")
  when credit = set_payment_type(payment_type = "CREDIT")
  when acknowledged = answer(button = "OK")
  when bad_date = type(item = "date-shipped", value = "1990-01-01")
  when date_refused = save()
  when after_refusal = click(item = "customer-id")
  when fixed_date = type(item = "date-shipped", value = "1992-10-01")
  when saved = save()
  then {
    assert steps.refused.messages == ["Invalid Customer Id!"]
    assert steps.refused.focus.item == "customer-id"
    assert steps.tabbed.messages == ["Function Not Allowed"]
    assert steps.credit.alert.text == "This customer must pay cash!"
    assert steps.acknowledged.order["payment-type"] == "CASH"
    assert steps.date_refused.messages == ["Ship date is before order date!"]
    assert result.order["date-shipped"] == "1992-10-01"
  }
}
```

```parity
proof orders-end-to-end.create-multiline {
  when entered = enter_query()
  when criteria = type(item = "order-id", value = 104)
  when queried = execute_query()
  when created = new_order()
  when customer = type(item = "customer-id", value = 201)
  when validated = click(item = "date-ordered")
  when ordered = type(item = "date-ordered", value = "2026-01-01")
  when shipped_date = type(item = "date-shipped", value = "2026-01-08")
  when payment = set_payment_type(payment_type = "CREDIT")
  when filled = toggle(item = "order-filled")
  when product_one = pick_product(line = 1, product_id = 30421)
  when quantity_one = type(item = "quantity", line = 1, value = 3)
  when shipped_one = type(item = "shipped", line = 1, value = 2)
  when total_one = click(item = "price", line = 1)
  when product_two = pick_product(line = 2, product_id = 40422)
  when quantity_two = type(item = "quantity", line = 2, value = 4)
  when shipped_two = type(item = "shipped", line = 2, value = 3)
  when saved = save()
  then {
    assert steps.created.order["date-ordered"] == "today"
    assert steps.total_one.total == "32.00"
    assert result.order["order-id"] == 50000
    assert result.lines[1]["item-id"] == 2
    assert result.total == "182.00"
    assert result.messages == ["FRM-40400: Transaction complete: 3 records applied and saved."]
  }
}
```

```parity
proof orders-end-to-end.dirty-clear-cancel-exit {
  when entered = enter_query()
  when criteria = type(item = "order-id", value = 105)
  when queried = execute_query()
  when changed = type(item = "customer-id", value = 201)
  when validated = click(item = "date-shipped")
  when clear_prompt = clear()
  when retained = answer(button = "Cancel")
  when filled = toggle(item = "order-filled")
  when cancel_refused = cancel()
  when after_refusal = click(item = "customer-id")
  when clear_again = clear()
  when discarded = answer(button = "No")
  when exit_prompt = exit()
  when declined = answer(button = "No")
  when after_decline = click(item = "order-id")
  when exit_again = exit()
  when closed = answer(button = "Yes")
  then {
    assert steps.clear_prompt.alert.buttons == ["Yes", "No", "Cancel"]
    assert steps.retained.order["customer-id"] == 201
    assert steps.cancel_refused.messages == ["Function Not Allowed"]
    assert steps.discarded.order == {"date-ordered": "today", "order-filled": "N", "payment-type": "CASH"}
    assert steps.exit_prompt.alert.text == "Do you really want to leave the form?"
    assert result.closed == true
  }
}
```

```parity
proof orders-end-to-end.duplicate-delete-master {
  when entered = enter_query()
  when criteria = type(item = "order-id", value = 106)
  when queried = execute_query()
  when duplicated = duplicate_order()
  when saved = save()
  when delete_prompt = delete_order()
  when declined = answer(button = "No")
  when after_decline = click(item = "customer-id")
  when delete_again = delete_order()
  when accepted = answer(button = "Yes")
  when deleted = save()
  then {
    assert steps.duplicated.lines == []
    assert !("order-id" in steps.duplicated.order)
    assert steps.saved.order["order-id"] == 50000
    assert steps.declined.order["order-id"] == 50000
    assert result.order["order-id"] == 106
    assert result.messages == ["FRM-40400: Transaction complete: 1 records applied and saved."]
  }
}
```
