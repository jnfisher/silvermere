// The settings filter: hides the settings whose key and description lack the typed text,
// and the sections left with none.
(function () {
  var box = document.getElementById("filter");
  if (!box) return;
  box.addEventListener("input", function () {
    var want = box.value.trim().toLowerCase();
    document.querySelectorAll("section").forEach(function (section) {
      var shown = 0;
      section.querySelectorAll(".setting").forEach(function (row) {
        var hit = !want || row.textContent.toLowerCase().indexOf(want) !== -1;
        row.hidden = !hit;
        if (hit) shown += 1;
      });
      section.hidden = shown === 0;
    });
  });
})();
