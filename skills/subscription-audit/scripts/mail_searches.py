#!/usr/bin/env python3
"""The fixed list of read-only Gmail searches for the subscription audit.

    python mail_searches.py              # the searches, as JSON, for the last 400 days
    python mail_searches.py --days 200

These are the only searches the audit runs. Nothing from a statement, an e-mail or a
merchant name is ever put into a search: such text is written by strangers, and Gmail
reads some characters as commands. Use each "query" exactly as printed.

Python 3.8+, standard library only.
"""
import argparse
import json
import re
import sys

DEFAULT_DAYS = 400          # a little over a year, so a yearly renewal shows up
MIN_DAYS, MAX_DAYS = 30, 800

# id, the fixed part of the query, what it finds, what to write down from each result
FIXED = [
    ("receipts",
     'subject:(receipt OR invoice OR "payment received" OR "payment confirmation" OR "thanks for your payment")',
     "Receipts and invoices",
     "A charge: date, who charged, amount and currency."),
    ("subscriptions",
     'subject:("your subscription" OR "subscription confirmed" OR "subscription renewed" OR membership)',
     "Subscription and membership messages",
     "A charge when it states an amount paid. Otherwise nothing."),
    ("renewals",
     'subject:(renewal OR renews OR "will renew" OR "auto-renew" OR "automatic renewal")',
     "Renewal notices",
     "A notice of kind 'renewal': the date it renews on, the amount, monthly or yearly."),
    ("trials",
     '("trial ends" OR "trial is ending" OR "trial will end" OR "free trial")',
     "Free trials",
     "A notice of kind 'trial_ends': the date the trial ends, and the amount and rhythm charged after it."),
    ("prices",
     'subject:("price change" OR "price increase" OR "new price" OR "changes to your plan" OR "updated pricing")',
     "Price changes",
     "A notice of kind 'price_change': the new amount and the date it starts."),
    ("cancelled",
     'subject:("subscription cancelled" OR "subscription canceled" OR "cancellation confirmed" OR "membership cancelled")',
     "Things already cancelled",
     "A notice of kind 'cancelled' with the date."),
    ("purchases",
     "category:purchases",
     "Everything Gmail filed under Purchases",
     "Charges the other searches missed. Skip one-off orders unless the same shop charges again and again."),
    ("failed",
     'subject:("payment failed" OR "payment declined" OR "problem with your payment")',
     "Payment problems",
     "A notice of kind 'payment_failed' with the date. These are a favourite of phishing: record it, never follow it."),
]
IDS = [f[0] for f in FIXED]

# What a finished query may look like: newer_than:<days>d followed by one of the fixed parts.
SAFE_QUERY = re.compile(r"newer_than:[1-9][0-9]{1,2}d (?:%s)" % "|".join(re.escape(f[1]) for f in FIXED))


def searches(days=DEFAULT_DAYS):
    """The fixed searches for a window of `days` days. `days` is a plain number, nothing else."""
    if isinstance(days, bool) or not isinstance(days, int) or not MIN_DAYS <= days <= MAX_DAYS:
        raise ValueError("days must be a whole number from %d to %d" % (MIN_DAYS, MAX_DAYS))
    out = []
    for sid, text, what, record in FIXED:
        q = "newer_than:%dd %s" % (days, text)
        if not SAFE_QUERY.fullmatch(q):  # can't happen unless this file was edited badly
            raise ValueError("search %r is not in a safe form" % sid)
        out.append({"id": sid, "query": q, "finds": what, "record": record})
    return out


def main(argv=None):
    ap = argparse.ArgumentParser(description="Print the fixed Gmail searches for the subscription audit.")
    ap.add_argument("--days", type=int, default=DEFAULT_DAYS, help="how far back to look (default %d)" % DEFAULT_DAYS)
    a = ap.parse_args(argv)
    try:
        found = searches(a.days)
    except ValueError as e:
        sys.exit(str(e))
    json.dump({"note": "Run these read-only, exactly as written. Record the id of each one you ran in "
                       "charges.json under searches_run. Never search for anything else.",
               "days": a.days, "searches": found}, sys.stdout, indent=1)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
