import type { ValidatePayloadParams } from "./generated/ts-client/index.js";

const accepted: ValidatePayloadParams = {
  payload: {
    array: ["right"],
    direct: "b",
    enum_intersection: "blue",
    object: {
      intersection: "nested-right",
      union: "nested-b",
    },
  },
};

void accepted;

const rejected: ValidatePayloadParams = {
  payload: {
    array: [
      "right",
      // @ts-expect-error nested allOf requires the const right branch.
      "left",
    ],
    // @ts-expect-error allOf requires the const blue branch.
    direct: "a",
    // @ts-expect-error allOf requires the const blue branch.
    enum_intersection: "red",
    object: {
      // @ts-expect-error nested allOf requires the const nested-right branch.
      intersection: "nested-left",
      // @ts-expect-error nested union must reject values outside its declared branches.
      union: "nested-c",
    },
  },
};

void rejected;
