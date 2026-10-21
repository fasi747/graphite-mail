import { describe, expect, it } from "vitest";
import { dataFolderFrom, joinPath, separatorOf } from "./paths";

describe("data folder paths", () => {
  it("joins with the separator the picked folder uses", () => {
    expect(joinPath("/home/ann/Documents", "graphite-mail")).toBe("/home/ann/Documents/graphite-mail");
    expect(joinPath("/home/ann/Documents/", "graphite-mail")).toBe("/home/ann/Documents/graphite-mail");
    expect(joinPath("C:\\Users\\Ann\\Documents", "graphite-mail")).toBe("C:\\Users\\Ann\\Documents\\graphite-mail");
    expect(joinPath("C:\\Users\\Ann\\Documents\\", "graphite-mail")).toBe("C:\\Users\\Ann\\Documents\\graphite-mail");
    expect(joinPath("\\\\server\\share\\mail", "graphite-mail")).toBe("\\\\server\\share\\mail\\graphite-mail");
    expect(separatorOf("D:/mixed/style")).toBe("\\");
  });

  it("adds the app folder once, on both platforms", () => {
    expect(dataFolderFrom("/data")).toBe("/data/graphite-mail");
    expect(dataFolderFrom("/data/graphite-mail")).toBe("/data/graphite-mail");
    expect(dataFolderFrom("/data/graphite-mail/")).toBe("/data/graphite-mail");
    expect(dataFolderFrom("C:\\Mail")).toBe("C:\\Mail\\graphite-mail");
    expect(dataFolderFrom("C:\\Mail\\graphite-mail")).toBe("C:\\Mail\\graphite-mail");
    expect(dataFolderFrom("C:\\Mail\\graphite-mail-old")).toBe("C:\\Mail\\graphite-mail-old\\graphite-mail");
  });
});
