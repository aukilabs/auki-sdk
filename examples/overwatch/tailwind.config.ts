import type { Config } from "tailwindcss";

export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        ink: "#101418",
        panel: "#171d22",
        line: "#29323a",
        signal: "#f97316",
      },
      borderRadius: {
        control: "8px",
      },
    },
  },
} satisfies Config;
