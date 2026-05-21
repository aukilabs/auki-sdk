# Parking lot — auki-uniffi-test Python package

Open questions for the UniFFI-generated Python package. Cross-package Python binding questions live in [`../parking_lot.md`](../parking_lot.md).

---

## Wheel policy

The package root can be used directly from source, but native-library wheel publication still needs a policy decision. Preferred direction: build one wheel per platform tag rather than publishing a single universal wheel that contains every native library.
