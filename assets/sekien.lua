-- sekien.lua: pandoc Lua filter for sekien
--
-- Converts RawBlock("html", <svg...>) nodes produced by sekien
-- into Image nodes pointing to temporary SVG files.
-- This allows PDF engines that drop raw HTML (e.g. typst) to include the SVG.
--
-- Note for Typst users:
--   Typst restricts file access to the project root by default.
--   When using this filter with Typst, you must grant access to /tmp:
--   pandoc ... --pdf-engine=typst --pdf-engine-opt=--root=/

function RawBlock(el)
  if el.format == "html" and el.text:match("^%s*<svg") then
    local path = os.tmpname() .. ".svg"
    local f = io.open(path, "w")
    if f then
      f:write(el.text)
      f:close()
    end
    return pandoc.Para({pandoc.Image({}, path)})
  end
end
