local count = 0
local saved = banter.storage.get("count")
if saved then
  count = tonumber(saved) or 0
end

banter.render(function()
  if banter.ui.begin("stage.header.actions") then
    banter.ui.text(tostring(count))
    if banter.ui.button("Ping") then
      count = count + 1
      banter.storage.set("count", tostring(count))
      banter.log("info", "ping " .. tostring(count))
    end
    banter.ui.close()
  end
end)
