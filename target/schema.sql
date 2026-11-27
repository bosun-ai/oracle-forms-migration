-- Summit order-entry tables. Business data is loaded separately.
CREATE TABLE s_region (
    id numeric(7,0) PRIMARY KEY,
    name varchar(50) NOT NULL UNIQUE
);
CREATE TABLE s_dept (
    id numeric(7,0) PRIMARY KEY,
    name varchar(25) NOT NULL,
    region_id numeric(7,0) REFERENCES s_region(id),
    UNIQUE (name, region_id)
);
CREATE TABLE s_title (title varchar(25) PRIMARY KEY);
CREATE TABLE s_emp (
    id numeric(7,0) PRIMARY KEY,
    last_name varchar(25) NOT NULL,
    first_name varchar(25),
    userid varchar(8) UNIQUE,
    start_date date,
    comments varchar(255),
    manager_id numeric(7,0) REFERENCES s_emp(id) DEFERRABLE INITIALLY DEFERRED,
    title varchar(25) REFERENCES s_title(title),
    dept_id numeric(7,0) REFERENCES s_dept(id),
    salary numeric(11,2),
    commission_pct numeric(4,2) CHECK (commission_pct IN (10,12.5,15,17.5,20))
);
CREATE TABLE s_customer (
    id numeric(7,0) PRIMARY KEY,
    name varchar(50) NOT NULL,
    phone varchar(25),
    address varchar(400),
    city varchar(30),
    state varchar(20),
    country varchar(30),
    zip_code varchar(75),
    credit_rating varchar(9) CHECK (credit_rating IN ('EXCELLENT','GOOD','POOR')),
    sales_rep_id numeric(7,0) REFERENCES s_emp(id),
    region_id numeric(7,0) REFERENCES s_region(id),
    comments varchar(255)
);
CREATE TABLE s_product (
    id numeric(7,0) PRIMARY KEY,
    name varchar(50) NOT NULL UNIQUE,
    short_desc varchar(255),
    longtext_id numeric(7,0),
    image_id numeric(7,0),
    suggested_whlsl_price numeric(11,2),
    whlsl_units varchar(25)
);
CREATE SEQUENCE s_ord_id START WITH 50000;
CREATE TABLE s_ord (
    id numeric(7,0) PRIMARY KEY,
    customer_id numeric(7,0) NOT NULL REFERENCES s_customer(id),
    date_ordered date,
    date_shipped date,
    sales_rep_id numeric(7,0) REFERENCES s_emp(id),
    total numeric(11,2),
    payment_type varchar(6) CHECK (payment_type IN ('CASH','CREDIT')),
    order_filled varchar(1) CHECK (order_filled IN ('Y','N'))
);
CREATE TABLE s_item (
    ord_id numeric(7,0) NOT NULL REFERENCES s_ord(id),
    item_id numeric(7,0) NOT NULL,
    product_id numeric(7,0) NOT NULL REFERENCES s_product(id),
    price numeric(11,2),
    quantity numeric(9,0),
    quantity_shipped numeric(9,0),
    PRIMARY KEY (ord_id, item_id),
    UNIQUE (ord_id, product_id)
);
CREATE TABLE s_item_audit (a numeric);
CREATE FUNCTION audit_s_item_insert() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO s_item_audit VALUES (1);
    RETURN NEW;
END;
$$;
CREATE TRIGGER audit_s_item AFTER INSERT ON s_item
    FOR EACH ROW EXECUTE FUNCTION audit_s_item_insert();
