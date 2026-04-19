-- sekien.lua: pandoc Lua filter for sekien
--
-- Converts RawBlock("html", <svg...>) nodes produced by sekien
-- into Image nodes pointing to temporary SVG files.
-- This allows PDF engines that drop raw HTML (e.g. typst) to include the SVG.
--
-- Installation:
--   sekien --print-lua-filter > $(pandoc --version | grep 'User data' | awk '{print $3}')/filters/sekien.lua
--
-- Usage:
--   pandoc input.md -o output.pdf --pdf-engine=typst --filter sekien --lua-filter sekien

function RawBlock(el)
  if el.format == "html" and el.text:match("^%s*<svg") then
    local path = os.tmpname() .. ".svg"
    local f = io.open(path, "w")
    f:write(el.text)
    f:close()
    return pandoc.Para({pandoc.Image({}, path)})
  end
end
