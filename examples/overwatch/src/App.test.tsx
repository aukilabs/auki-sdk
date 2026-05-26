import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { App } from "./App";

describe("App", () => {
  it("renders the operator shell as the first screen", () => {
    render(<App />);

    expect(screen.getByRole("banner")).toHaveTextContent("Auki Overwatch");
    expect(screen.getByLabelText(/Discovery URL/i)).toHaveValue("http://127.0.0.1:8091");
    expect(screen.getByLabelText(/Domain name/i)).toHaveValue("overwatch");
    expect(screen.getByText("No remote peers")).toBeInTheDocument();
  });
});
